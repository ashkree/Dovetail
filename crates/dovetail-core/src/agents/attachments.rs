//! Files behind chat-bar image attachments, kept under `attachments/`.
//!
//! Two jobs live here, both plain filesystem work for the core:
//!
//! - **Pasted paths.** A terminal delivers a pasted or dropped file as text: a bare path, a
//!   `file://` URI, a `~` path, or a path in shell quotes or with backslash escapes. When that
//!   text names an existing PNG, JPEG, GIF or WebP file, [`paste_text`] copies it into
//!   `attachments/` and answers with an attachment, exactly as `Ctrl-V` does for a clipboard image.
//!   Anything else stays text.
//! - **Cleanup** (policy, decided with task #8): a file a live agent's conversation names is never
//!   deleted. An agent's files go when it is archived ([`remove`]); at startup [`sweep`] removes
//!   files older than [`MAX_AGE`] that no live agent in any project names. Nothing outside
//!   `attachments/` is ever touched.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use dovetail_api::agents::conversation::{ImageType, MAX_ATTACHMENT_BYTES};
use dovetail_api::app::Paste;

use crate::clipboard::{self, Clip, ClipError};

/// How old an unreferenced attachment must be before the startup sweep removes it.
pub const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// File extensions that may name an accepted image; the file's own bytes decide the format.
const EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

/// Turns pasted text into the answer for the chat bar: an attachment when the text is a path to
/// an accepted image (copied under `dir`), else the text itself. A path to an image that is too
/// large, unreadable or not really an image also stays text.
pub fn paste_text(text: String, dir: &Path, home: Option<&Path>) -> Paste {
    let Some(path) = image_path(&text, home) else {
        return Paste::Text(text);
    };
    match read_image(&path) {
        Ok(clip) => clipboard::paste_from(Ok(clip), dir),
        Err(e) => {
            tracing::debug!("pasted path {} is not attached: {e}", path.display());
            Paste::Text(text)
        }
    }
}

/// The format the file's first bytes show, if it is one of the accepted ones.
pub(crate) fn sniff(bytes: &[u8]) -> Option<ImageType> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageType::Png)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(ImageType::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(ImageType::Gif)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageType::Webp)
    } else {
        None
    }
}

fn read_image(path: &Path) -> Result<Clip, ClipError> {
    let meta = std::fs::metadata(path).map_err(|e| ClipError::Failed(e.to_string()))?;
    if !meta.is_file() {
        return Err(ClipError::Failed("not a file".into()));
    }
    if meta.len() > MAX_ATTACHMENT_BYTES {
        return Err(ClipError::TooLarge(meta.len()));
    }
    let bytes = std::fs::read(path).map_err(|e| ClipError::Failed(e.to_string()))?;
    // The extension only filtered; the bytes decide the format (and refuse a mislabelled file).
    match sniff(&bytes) {
        Some(found) => clipboard::check_image(found, bytes),
        None => Err(ClipError::Unsupported(path.display().to_string())),
    }
}

/// The absolute path a pasted text names, when it is a single path to a file with an image
/// extension that exists. Handles `~`, `file://` URIs, shell quotes and backslash escapes.
pub fn image_path(text: &str, home: Option<&Path>) -> Option<PathBuf> {
    let path = parse_path(text, home)?;
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    (EXTENSIONS.contains(&ext.as_str()) && path.is_file()).then_some(path)
}

/// Reads one absolute path out of terminal paste text, without touching the filesystem.
fn parse_path(text: &str, home: Option<&Path>) -> Option<PathBuf> {
    let text = text.trim();
    if text.is_empty() || text.contains(['\n', '\r', '\0']) {
        return None;
    }
    let plain = if let Some(rest) = text.strip_prefix("file://") {
        // `file:///x` or `file://localhost/x`; another host is not this machine's file.
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        if !rest.starts_with('/') {
            return None;
        }
        percent_decode(rest)?
    } else {
        unquote(text)?
    };
    if let Some(rest) = plain.strip_prefix('~') {
        let home = home?;
        return match rest {
            "" => Some(home.to_path_buf()),
            _ => Some(home.join(rest.strip_prefix('/')?)),
        };
    }
    plain.starts_with('/').then(|| PathBuf::from(plain))
}

/// Undoes shell quoting: a whole-text pair of single or double quotes, or backslash escapes.
/// Spaces and quotes inside unquoted text are kept: some terminals deliver a dropped path
/// unescaped, and the filesystem check settles whether the whole text is a file.
fn unquote(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let (first, last) = (*bytes.first()?, *bytes.last()?);
    if text.len() >= 2 && first == last && (first == b'\'' || first == b'"') {
        let inner = &text[1..text.len() - 1];
        if inner.contains(first as char) && first == b'\'' {
            return None;
        }
        if first == b'"' {
            let mut out = String::new();
            let mut chars = inner.chars();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => match chars.next() {
                        Some(n @ ('"' | '\\' | '$' | '`')) => out.push(n),
                        Some(n) => {
                            out.push('\\');
                            out.push(n);
                        }
                        None => out.push('\\'),
                    },
                    '"' => return None,
                    c => out.push(c),
                }
            }
            return Some(out);
        }
        return Some(inner.to_string());
    }
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push(chars.next()?),
            c => out.push(c),
        }
    }
    Some(out)
}

/// Decodes `%XX` escapes; `None` when the result is not UTF-8 or an escape is malformed.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Deletes the given attachment files, but only those directly inside `dir`. Missing files are
/// fine. Returns how many were removed.
pub fn remove(dir: &Path, paths: &[PathBuf]) -> usize {
    let mut removed = 0;
    for path in paths {
        if path.parent() != Some(dir) {
            continue;
        }
        match std::fs::remove_file(path) {
            Ok(()) => removed += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!("removing attachment {}: {e}", path.display()),
        }
    }
    removed
}

/// Deletes regular files in `dir` last modified more than `max_age` before `now` that are not in
/// `keep`. Subfolders and files with a recent mtime stay. Returns how many were removed.
pub fn sweep(dir: &Path, now: SystemTime, max_age: Duration, keep: &HashSet<PathBuf>) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        let old = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age > max_age);
        if meta.is_file() && old && !keep.contains(&path) && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n....";

    fn home() -> PathBuf {
        PathBuf::from("/home/u")
    }

    #[test]
    fn terminal_path_forms_are_read() {
        let h = home();
        let p = |t: &str| parse_path(t, Some(&h));
        assert_eq!(p("/tmp/a.png"), Some("/tmp/a.png".into()));
        assert_eq!(p("  /tmp/a.png\n"), Some("/tmp/a.png".into()));
        assert_eq!(p("~/shots/a.png"), Some("/home/u/shots/a.png".into()));
        assert_eq!(p("file:///tmp/a%20b.png"), Some("/tmp/a b.png".into()));
        assert_eq!(p("file://localhost/tmp/a.png"), Some("/tmp/a.png".into()));
        assert_eq!(p("file://otherhost/tmp/a.png"), None);
        assert_eq!(p("'/tmp/a b.png'"), Some("/tmp/a b.png".into()));
        assert_eq!(p("\"/tmp/a b.png\""), Some("/tmp/a b.png".into()));
        assert_eq!(p("/tmp/a\\ b\\(1\\).png"), Some("/tmp/a b(1).png".into()));
    }

    #[test]
    fn text_that_is_not_one_path_is_not_read() {
        let h = home();
        let p = |t: &str| parse_path(t, Some(&h));
        assert_eq!(p(""), None);
        assert_eq!(p("look at /tmp/a.png"), None);
        assert_eq!(p("/tmp/a.png\n/tmp/b.png"), None);
        assert_eq!(p("relative/a.png"), None);
        assert_eq!(p("~other/a.png"), None);
        assert_eq!(p("file:///tmp/%zz.png"), None);
        assert_eq!(p("'/tmp/it's.png'"), None);
        assert_eq!(parse_path("~/a.png", None), None);
    }

    #[test]
    fn an_existing_image_path_is_attached_and_copied() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("my shot.PNG");
        std::fs::write(&src, PNG).unwrap();
        let dir = tmp.path().join("attachments");
        for text in [
            src.display().to_string(),
            format!("'{}'", src.display()),
            format!("file://{}", src.display().to_string().replace(' ', "%20")),
            src.display().to_string().replace(' ', "\\ "),
        ] {
            match paste_text(text.clone(), &dir, None) {
                Paste::Image(a) => {
                    assert_eq!(a.path.parent(), Some(dir.as_path()), "{text}");
                    assert_eq!(std::fs::read(&a.path).unwrap(), PNG);
                    assert_eq!(a.size, PNG.len() as u64);
                }
                other => panic!("{text}: {other:?}"),
            }
        }
        assert!(src.exists(), "the original is left alone");
    }

    #[test]
    fn home_paths_use_the_given_home() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.jpg"), b"\xff\xd8\xff\xe0....").unwrap();
        let dir = tmp.path().join("att");
        let paste = paste_text("~/a.jpg".into(), &dir, Some(tmp.path()));
        assert!(matches!(paste, Paste::Image(_)), "{paste:?}");
    }

    #[test]
    fn everything_else_stays_text() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("att");
        let img = |name: &str, bytes: &[u8]| {
            let p = tmp.path().join(name);
            std::fs::write(&p, bytes).unwrap();
            p.display().to_string()
        };
        let cases = [
            "hello world".to_string(),
            tmp.path().join("missing.png").display().to_string(),
            img("notes.txt", PNG),
            img("fake.png", b"not an image at all"),
            tmp.path().display().to_string(),
            img("big.png", &vec![0; MAX_ATTACHMENT_BYTES as usize + 1]),
            img("bmp.bmp", b"BM...."),
        ];
        for text in cases {
            assert_eq!(paste_text(text.clone(), &dir, None), Paste::Text(text));
        }
        assert!(!dir.exists(), "nothing was saved");
    }

    fn aged(path: &Path, age: Duration) {
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
    }

    #[test]
    fn the_sweep_removes_old_unreferenced_files_only() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let day = Duration::from_secs(86_400);
        let make = |name: &str, age: Duration| {
            let p = dir.join(name);
            std::fs::write(&p, PNG).unwrap();
            aged(&p, age);
            p
        };
        let old = make("old.png", 8 * day);
        let kept = make("kept.png", 30 * day);
        let fresh = make("fresh.png", day);
        std::fs::create_dir(dir.join("sub")).unwrap();
        let keep = HashSet::from([kept.clone()]);
        let n = sweep(dir, SystemTime::now(), MAX_AGE, &keep);
        assert_eq!(n, 1);
        assert!(!old.exists());
        assert!(kept.exists() && fresh.exists() && dir.join("sub").exists());
        assert_eq!(
            sweep(&dir.join("missing"), SystemTime::now(), MAX_AGE, &keep),
            0
        );
    }

    #[test]
    fn remove_deletes_only_files_inside_the_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("att");
        std::fs::create_dir(&dir).unwrap();
        let inside = dir.join("a.png");
        let outside = tmp.path().join("b.png");
        std::fs::write(&inside, PNG).unwrap();
        std::fs::write(&outside, PNG).unwrap();
        let gone = dir.join("gone.png");
        let n = remove(&dir, &[inside.clone(), outside.clone(), gone]);
        assert_eq!(n, 1);
        assert!(!inside.exists());
        assert!(outside.exists());
    }

    #[test]
    fn sniff_identifies_accepted_formats_and_rejects_others() {
        assert_eq!(sniff(PNG), Some(ImageType::Png));
        assert_eq!(sniff(b"\xff\xd8\xff\xe0"), Some(ImageType::Jpeg));
        assert_eq!(sniff(b"GIF87a.."), Some(ImageType::Gif));
        assert_eq!(sniff(b"GIF89a.."), Some(ImageType::Gif));
        assert_eq!(sniff(b"RIFF\x00\x00\x00\x00WEBP"), Some(ImageType::Webp));
        assert_eq!(sniff(b"RIFF\x00\x00\x00\x00AVI "), None);
        assert_eq!(sniff(b"RIFFWEBP"), None);
        assert_eq!(sniff(b""), None);
        assert_eq!(sniff(b"BM...."), None);
    }
}
