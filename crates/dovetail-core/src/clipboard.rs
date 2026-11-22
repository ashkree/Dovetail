//! The system clipboard, for Ctrl-V in a chat bar ([`dovetail_api::AppCommand::Paste`])
//! and for copying text ([`dovetail_api::AppCommand::Copy`]). Terminals cannot carry an
//! image through a paste, so the core asks the clipboard itself: `wl-paste`/`wl-copy` on
//! Wayland, `xclip` on X11. Both run as short-lived child processes that are always
//! reaped; no clipboard crate is used. The service calls [`paste`] on a blocking thread, off its
//! main loop.
//!
//! An image (PNG, JPEG, GIF or WebP up to 5 MB) is saved under `attachments/` and becomes an
//! attachment; anything else that is text is pasted at the cursor. Nothing is resized or
//! converted.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dovetail_api::{
    app::Paste,
    conversation::{Attachment, ImageType, MAX_ATTACHMENT_BYTES, attachment_limit_text, size_text},
};

/// How long one clipboard tool may take before it is killed.
const TIMEOUT: Duration = Duration::from_secs(3);

/// What the clipboard held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clip {
    /// An accepted image.
    Image {
        /// Its format, from its first bytes.
        media_type: ImageType,
        /// The file bytes.
        bytes: Vec<u8>,
    },
    /// Text to paste at the cursor.
    Text(String),
}

/// Why nothing could be pasted. `Display` is the short notice shown in the status bar.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClipError {
    /// Neither `wl-paste` nor `xclip` is installed.
    #[error("can't read the clipboard: install wl-clipboard or xclip")]
    NoTool,
    /// The clipboard holds nothing.
    #[error("the clipboard is empty")]
    Empty,
    /// The clipboard holds an image in a format Claude does not take.
    #[error("can't attach {0}: only PNG, JPEG, GIF or WebP")]
    Unsupported(String),
    /// The image is over [`MAX_ATTACHMENT_BYTES`].
    #[error("image is {}: the limit is {}", size_text(*.0), attachment_limit_text())]
    TooLarge(u64),
    /// The tool failed or timed out.
    #[error("clipboard read failed: {0}")]
    Failed(String),
}

/// Reads the clipboard and, for an image, saves it under `dir`. Blocks for up to a few seconds;
/// call it off the core's loop.
pub fn paste(dir: &Path) -> Paste {
    paste_from(read(), dir)
}

/// Turns a clipboard read into the answer for the UI, saving an image under `dir`.
pub fn paste_from(clip: Result<Clip, ClipError>, dir: &Path) -> Paste {
    match clip {
        Ok(Clip::Text(text)) => Paste::Text(text),
        Ok(Clip::Image { media_type, bytes }) => match save(dir, media_type, &bytes) {
            Ok(attachment) => Paste::Image(attachment),
            Err(e) => {
                tracing::warn!("saving a pasted image failed: {e}");
                Paste::Refused(format!("could not save the image: {e}"))
            }
        },
        Err(e) => Paste::Refused(e.to_string()),
    }
}

/// What to fetch, given the MIME types the clipboard offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offer {
    /// An accepted image type; the string is the type exactly as offered, to ask for it.
    Image(ImageType, String),
    /// Only image types Claude does not take; the first one, for the notice.
    OtherImage(String),
    /// No image: paste text.
    Text,
}

/// Picks what to fetch from the offered MIME types: an accepted image first (in
/// [`ImageType::ALL`] order), then any other image (refused), else text.
pub fn choose<'a>(types: impl IntoIterator<Item = &'a str>) -> Offer {
    let types: Vec<&str> = types
        .into_iter()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    for wanted in ImageType::ALL {
        if let Some(offered) = types
            .iter()
            .find(|t| ImageType::from_mime(t) == Some(wanted))
        {
            return Offer::Image(wanted, (*offered).to_string());
        }
    }
    match types
        .iter()
        .find(|t| t.to_ascii_lowercase().starts_with("image/"))
    {
        Some(other) => Offer::OtherImage((*other).to_string()),
        None => Offer::Text,
    }
}

/// Checks fetched image bytes: not empty, at most 5 MB, and in an accepted format. The bytes'
/// own signature wins over the offered type, so the block sent to Claude names the right one;
/// bytes that are clearly another format are refused.
pub fn check_image(offered: ImageType, bytes: Vec<u8>) -> Result<Clip, ClipError> {
    if bytes.is_empty() {
        return Err(ClipError::Empty);
    }
    let size = bytes.len() as u64;
    if size > MAX_ATTACHMENT_BYTES {
        return Err(ClipError::TooLarge(size));
    }
    let media_type = match crate::attachments::sniff(&bytes) {
        Some(found) => found,
        None => return Err(ClipError::Unsupported(offered.mime().to_string())),
    };
    Ok(Clip::Image { media_type, bytes })
}

/// A clipboard tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    WlPaste,
    Xclip,
}

impl Tool {
    fn program(self) -> &'static str {
        match self {
            Tool::WlPaste => "wl-paste",
            Tool::Xclip => "xclip",
        }
    }

    fn list_args(self) -> Vec<&'static str> {
        match self {
            Tool::WlPaste => vec!["--list-types"],
            Tool::Xclip => vec!["-selection", "clipboard", "-t", "TARGETS", "-o"],
        }
    }

    fn fetch_args(self, mime: &str) -> Vec<String> {
        match self {
            Tool::WlPaste => vec!["--no-newline".into(), "--type".into(), mime.into()],
            Tool::Xclip => vec![
                "-selection".into(),
                "clipboard".into(),
                "-t".into(),
                mime.into(),
                "-o".into(),
            ],
        }
    }

    fn text_args(self) -> Vec<&'static str> {
        match self {
            // `text` asks wl-paste for any textual type.
            Tool::WlPaste => vec!["--no-newline", "--type", "text"],
            Tool::Xclip => vec!["-selection", "clipboard", "-o"],
        }
    }
}

/// Tools to try, the session's own display server first.
fn tools() -> Vec<Tool> {
    let set = |var: &str| std::env::var_os(var).is_some_and(|v| !v.is_empty());
    let mut tools = Vec::new();
    if set("WAYLAND_DISPLAY") {
        tools.push(Tool::WlPaste);
    }
    if set("DISPLAY") {
        tools.push(Tool::Xclip);
    }
    if tools.is_empty() {
        tools = vec![Tool::WlPaste, Tool::Xclip];
    }
    tools
}

/// Reads the clipboard: an image if it offers one, else its text.
pub fn read() -> Result<Clip, ClipError> {
    for tool in tools() {
        let listed = match run(tool.program(), &tool.list_args()) {
            Ok(out) => out,
            Err(RunError::NotFound) => continue,
            Err(e) => return Err(e.into()),
        };
        // An empty or failed listing means nothing is copied (wl-paste exits 1 then).
        let Some(listed) = listed else {
            return Err(ClipError::Empty);
        };
        let listing = String::from_utf8_lossy(&listed);
        return match choose(listing.lines()) {
            Offer::Image(kind, mime) => {
                let bytes =
                    run(tool.program(), &tool.fetch_args(&mime))?.ok_or(ClipError::Empty)?;
                check_image(kind, bytes)
            }
            Offer::OtherImage(mime) => Err(ClipError::Unsupported(mime)),
            Offer::Text => {
                let bytes = run(tool.program(), &tool.text_args())?.ok_or(ClipError::Empty)?;
                let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
                if text.is_empty() {
                    Err(ClipError::Empty)
                } else {
                    Ok(Clip::Text(text))
                }
            }
        };
    }
    Err(ClipError::NoTool)
}

/// Why text could not be copied. `Display` is the short notice shown in the status bar.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CopyError {
    /// Neither `wl-copy` nor `xclip` is installed.
    #[error("can't copy: install wl-clipboard or xclip")]
    NoTool,
    /// The tool failed or timed out.
    #[error("copying failed: {0}")]
    Failed(String),
}

/// Puts `text` on the clipboard. Both tools fork a child that keeps serving the clipboard and
/// let the parent exit once it has read its input; that parent is waited for (at most
/// [`TIMEOUT`]) and always reaped. Blocks; call it off the core's loop.
pub fn copy(text: &str) -> Result<(), CopyError> {
    for tool in tools() {
        let (program, args): (&str, &[&str]) = match tool {
            Tool::WlPaste => ("wl-copy", &[]),
            Tool::Xclip => ("xclip", &["-selection", "clipboard", "-i"]),
        };
        let mut child = match Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(CopyError::Failed(format!("{program}: {e}"))),
        };
        let written = child
            .stdin
            .take()
            .map(|mut stdin| stdin.write_all(text.as_bytes()));
        // `stdin` is dropped here, closing the pipe so the tool sees the end of its input.
        let deadline = Instant::now() + TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => {
                    reap(&mut child);
                    return Err(CopyError::Failed(format!("{program} timed out")));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                Err(e) => {
                    reap(&mut child);
                    return Err(CopyError::Failed(format!("{program}: {e}")));
                }
            }
        };
        if let Some(Err(e)) = written {
            return Err(CopyError::Failed(format!("{program}: {e}")));
        }
        return if status.success() {
            Ok(())
        } else {
            Err(CopyError::Failed(format!("{program} exited with {status}")))
        };
    }
    Err(CopyError::NoTool)
}

/// Why running one clipboard tool failed.
#[derive(Debug, thiserror::Error)]
enum RunError {
    #[error("not installed")]
    NotFound,
    #[error("output over the limit ({0} bytes)")]
    TooLarge(u64),
    #[error("{0}")]
    Failed(String),
}

impl From<RunError> for ClipError {
    fn from(e: RunError) -> Self {
        match e {
            RunError::NotFound => ClipError::NoTool,
            RunError::TooLarge(size) => ClipError::TooLarge(size),
            RunError::Failed(why) => ClipError::Failed(why),
        }
    }
}

/// Runs a clipboard tool and returns its stdout, or `None` when it exits unsuccessfully (which
/// both tools do when the clipboard is empty). Output beyond the 5 MB limit stops the read; a
/// tool that hangs is killed after [`TIMEOUT`]. The child is always reaped.
fn run<S: AsRef<std::ffi::OsStr>>(program: &str, args: &[S]) -> Result<Option<Vec<u8>>, RunError> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => RunError::NotFound,
            _ => RunError::Failed(format!("{program}: {e}")),
        })?;
    let Some(stdout) = child.stdout.take() else {
        reap(&mut child);
        return Err(RunError::Failed(format!("{program}: no output pipe")));
    };
    // Read on a thread so the timeout below can still fire. Reading one byte past the limit is
    // enough to know the image is too large; dropping the pipe then ends the tool.
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let result = stdout.take(MAX_ATTACHMENT_BYTES + 1).read_to_end(&mut buf);
        result.map(|_| buf)
    });
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() >= deadline => break None,
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => {
                reap(&mut child);
                return Err(RunError::Failed(format!("{program}: {e}")));
            }
        }
    };
    let Some(status) = status else {
        reap(&mut child);
        let _ = reader.join();
        return Err(RunError::Failed(format!("{program} timed out")));
    };
    let bytes = match reader.join() {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(e)) => return Err(RunError::Failed(format!("{program}: {e}"))),
        Err(_) => return Err(RunError::Failed(format!("{program}: reader panicked"))),
    };
    if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
        return Err(RunError::TooLarge(bytes.len() as u64));
    }
    Ok(status.success().then_some(bytes))
}

/// Kills and waits, so a child is never left as a zombie.
fn reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Saves a pasted image under `dir` (Dovetail's `attachments/` data folder) with a unique name.
pub fn save(dir: &Path, media_type: ImageType, bytes: &[u8]) -> io::Result<Attachment> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    std::fs::create_dir_all(dir)?;
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let name = format!(
        "{ms}-{}-{}.{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed),
        media_type.extension()
    );
    let path: PathBuf = dir.join(name);
    std::fs::write(&path, bytes)?;
    Ok(Attachment {
        path,
        media_type,
        size: bytes.len() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n....";

    #[test]
    fn an_accepted_image_type_wins_over_text() {
        let listing = "text/plain\nimage/png\nTARGETS\n";
        assert_eq!(
            choose(listing.lines()),
            Offer::Image(ImageType::Png, "image/png".into())
        );
        // Browsers offer several; PNG first, JPEG offered as image/jpg still counts.
        assert_eq!(
            choose(["image/gif", "image/jpg"]),
            Offer::Image(ImageType::Jpeg, "image/jpg".into())
        );
        assert_eq!(
            choose(["image/webp", "image/gif"]),
            Offer::Image(ImageType::Webp, "image/webp".into())
        );
    }

    #[test]
    fn other_image_types_are_refused_and_no_image_means_text() {
        assert_eq!(
            choose(["image/bmp", "text/plain"]),
            Offer::OtherImage("image/bmp".into())
        );
        assert_eq!(
            choose(["UTF8_STRING", "text/plain;charset=utf-8"]),
            Offer::Text
        );
        assert_eq!(choose([""]), Offer::Text);
        assert_eq!(
            ClipError::Unsupported("image/bmp".into()).to_string(),
            "can't attach image/bmp: only PNG, JPEG, GIF or WebP"
        );
        assert_eq!(
            ClipError::NoTool.to_string(),
            "can't read the clipboard: install wl-clipboard or xclip"
        );
    }

    #[test]
    fn images_over_5_mb_are_refused() {
        let big = MAX_ATTACHMENT_BYTES as usize + 1;
        let mut bytes = PNG.to_vec();
        bytes.resize(big, 0);
        assert_eq!(
            check_image(ImageType::Png, bytes),
            Err(ClipError::TooLarge(big as u64))
        );
        let mut at_limit = PNG.to_vec();
        at_limit.resize(MAX_ATTACHMENT_BYTES as usize, 0);
        assert!(check_image(ImageType::Png, at_limit).is_ok());
        assert_eq!(
            ClipError::TooLarge(big as u64).to_string(),
            "image is 5.0 MB: the limit is 5 MB"
        );
    }

    #[test]
    fn the_bytes_decide_the_format_and_garbage_is_refused() {
        let gif = b"GIF89a....".to_vec();
        assert_eq!(
            check_image(ImageType::Png, gif.clone()),
            Ok(Clip::Image {
                media_type: ImageType::Gif,
                bytes: gif
            })
        );
        assert_eq!(
            check_image(ImageType::Png, b"BM not a png".to_vec()),
            Err(ClipError::Unsupported("image/png".into()))
        );
        assert_eq!(
            check_image(ImageType::Png, Vec::new()),
            Err(ClipError::Empty)
        );
    }

    /// Manual check against the real clipboard tool: copy an image or some text, then run
    /// `cargo test -p dovetail-core real_clipboard -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads the system clipboard"]
    fn real_clipboard() {
        match read() {
            Ok(Clip::Image { media_type, bytes }) => {
                println!("image {} · {} bytes", media_type.mime(), bytes.len());
            }
            Ok(Clip::Text(text)) => println!("text, {} chars", text.chars().count()),
            Err(e) => println!("refused: {e}"),
        }
    }

    #[test]
    fn a_saved_image_lands_in_the_folder_with_its_extension() {
        let dir = tempfile::tempdir().expect("tempdir");
        let saved = save(dir.path(), ImageType::Png, PNG).expect("save");
        assert_eq!(saved.size, PNG.len() as u64);
        assert_eq!(saved.path.extension().and_then(|e| e.to_str()), Some("png"));
        assert_eq!(std::fs::read(&saved.path).expect("read back"), PNG);
    }

    #[test]
    fn a_paste_saves_images_and_passes_text_and_refusals_through() {
        let dir = tempfile::tempdir().expect("tempdir");
        let attachments = dir.path().join("attachments");
        let image = Ok(Clip::Image {
            media_type: ImageType::Png,
            bytes: PNG.to_vec(),
        });
        let Paste::Image(saved) = paste_from(image, &attachments) else {
            panic!("an image");
        };
        assert!(saved.path.starts_with(&attachments));
        assert_eq!(std::fs::read(&saved.path).expect("saved"), PNG);
        assert_eq!(
            paste_from(Ok(Clip::Text("hi".into())), &attachments),
            Paste::Text("hi".into())
        );
        assert_eq!(
            paste_from(Err(ClipError::Empty), &attachments),
            Paste::Refused("the clipboard is empty".into())
        );
    }
}
