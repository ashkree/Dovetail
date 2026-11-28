//! kitty's graphics protocol, the few commands the canvas needs, written by hand: an image is
//! sent once (PNG for the map, raw RGBA for the selection ring), then placed and re-placed with
//! a source rectangle as the view moves.
//!
//! Every command carries `q=2`, so kitty sends no replies: replies arrive on stdin, where the
//! key reader would take them for typing. Both images sit under the text (`z` < 0), so a
//! popup's text stays readable over them; the canvas hides them while a popup is open anyway.
//! [`Screen`] remembers what is on screen and writes only the difference.
//!
//! Reading mode's images ([`Figure`], ADR 0018) are sent by path (`t=f`): kitty reads the PNG
//! itself, so the UI never opens the file.

use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine as _;

use super::Picture;
use super::ring::Ring;
use super::view::Placement;

/// kitty's limit for one chunk of base64 payload.
const CHUNK: usize = 4096;
/// The map picture's image id. One map shows at a time, so fixed ids are enough.
pub const MAP_ID: u32 = 7_301;
/// The selection ring's image id.
pub const RING_ID: u32 = 7_302;
/// The map sits under the text, the ring over the map.
const MAP_Z: i32 = -2;
const RING_Z: i32 = -1;

/// Standard padded base64.
pub fn base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// One graphics command: `ESC _ G <control> ; <payload> ESC \`.
fn command(out: &mut Vec<u8>, control: &str, payload: &str) {
    out.extend_from_slice(b"\x1b_G");
    out.extend_from_slice(control.as_bytes());
    if !payload.is_empty() {
        out.push(b';');
        out.extend_from_slice(payload.as_bytes());
    }
    out.extend_from_slice(b"\x1b\\");
}

/// Sends `data` under `control`, base64 in chunks: every chunk but the last says `m=1`, and the
/// keys go on the first chunk only.
fn transmit(out: &mut Vec<u8>, control: &str, data: &[u8]) {
    let encoded = base64(data);
    let chunks: Vec<&[u8]> = encoded.as_bytes().chunks(CHUNK).collect();
    let last = chunks.len().saturating_sub(1);
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i < last);
        let keys = if i == 0 {
            format!("{control},m={more}")
        } else {
            format!("m={more}")
        };
        // Base64 is ASCII.
        command(out, &keys, std::str::from_utf8(chunk).unwrap_or_default());
    }
    if chunks.is_empty() {
        command(out, control, "");
    }
}

/// Sends a PNG as image `id`, replacing any image with that id.
pub fn send_png(out: &mut Vec<u8>, id: u32, png: &[u8]) {
    transmit(out, &format!("a=t,f=100,t=d,i={id},q=2"), png);
}

/// Sends raw RGBA pixels as image `id`.
pub fn send_rgba(out: &mut Vec<u8>, id: u32, width: u32, height: u32, rgba: &[u8]) {
    transmit(
        out,
        &format!("a=t,f=32,s={width},v={height},t=d,i={id},q=2"),
        rgba,
    );
}

/// Places the map picture: its source rectangle scaled into the placement's cells. The cursor
/// is saved and restored around it, so the UI's own cursor does not move.
pub fn place_map(out: &mut Vec<u8>, p: &Placement) {
    move_to(out, p.col, p.row);
    command(
        out,
        &format!(
            "a=p,i={MAP_ID},p=1,x={},y={},w={},h={},c={},r={},z={MAP_Z},C=1,q=2",
            p.src_x, p.src_y, p.src_w, p.src_h, p.cols, p.rows
        ),
        "",
    );
    out.extend_from_slice(b"\x1b8");
}

/// Places the ring at its own pixel size, offset inside its first cell.
pub fn place_ring(out: &mut Vec<u8>, r: &Ring) {
    move_to(out, r.col, r.row);
    command(
        out,
        &format!(
            "a=p,i={RING_ID},p=1,X={},Y={},z={RING_Z},C=1,q=2",
            r.x_off, r.y_off
        ),
        "",
    );
    out.extend_from_slice(b"\x1b8");
}

/// Saves the cursor and moves it to a 0-based cell.
fn move_to(out: &mut Vec<u8>, col: u16, row: u16) {
    out.extend_from_slice(format!("\x1b7\x1b[{};{}H", row + 1, col + 1).as_bytes());
}

/// An image of the reading column to place after the frame, in screen cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Figure {
    /// kitty image id.
    pub id: u32,
    /// The PNG file, which kitty reads itself.
    pub path: PathBuf,
    /// Left column.
    pub col: u16,
    /// Top row.
    pub row: u16,
    /// Width in cells.
    pub cols: u16,
    /// Height in cells.
    pub rows: u16,
}

/// Sends a PNG file as image `id`: kitty reads it from `path`.
pub fn send_png_file(out: &mut Vec<u8>, id: u32, path: &std::path::Path) {
    #[cfg(unix)]
    let bytes = path.as_os_str().as_bytes();
    #[cfg(not(unix))]
    let bytes = path.as_os_str().as_encoded_bytes();
    transmit(out, &format!("a=t,f=100,t=f,i={id},q=2"), bytes);
}

/// Places a figure scaled into its cells, under the text.
pub fn place_figure(out: &mut Vec<u8>, f: &Figure) {
    move_to(out, f.col, f.row);
    command(
        out,
        &format!(
            "a=p,i={},p=1,c={},r={},z={MAP_Z},C=1,q=2",
            f.id, f.cols, f.rows
        ),
        "",
    );
    out.extend_from_slice(b"\x1b8");
}

/// Takes an image's placements off the screen, keeping its data for a later placement.
pub fn hide(out: &mut Vec<u8>, id: u32) {
    command(out, &format!("a=d,d=i,i={id},q=2"), "");
}

/// Deletes an image and frees its data.
pub fn delete(out: &mut Vec<u8>, id: u32) {
    command(out, &format!("a=d,d=I,i={id},q=2"), "");
}

/// What the canvas wants on screen after a frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    /// The picture; a new `Arc` is a new picture.
    pub picture: Arc<Picture>,
    /// Where it goes.
    pub placement: Placement,
    /// The selection ring, if a selected node is in view.
    pub ring: Option<Ring>,
}

/// What is on the terminal now, so each frame writes only what changed.
#[derive(Default)]
pub struct Screen {
    /// The picture sent as [`MAP_ID`].
    sent: Option<Arc<Picture>>,
    placed: Option<Placement>,
    ring: Option<Ring>,
    /// Reading-mode figures sent, by id, with the file each was sent from.
    figures_sent: HashMap<u32, PathBuf>,
    /// Reading-mode figures on screen, by id.
    figures_placed: HashMap<u32, Figure>,
}

impl Screen {
    /// Brings the terminal to `want`, appending the commands to `out`.
    pub fn sync(&mut self, want: Option<&Scene>, out: &mut Vec<u8>) {
        let Some(want) = want else {
            if self.placed.take().is_some() {
                hide(out, MAP_ID);
            }
            if self.ring.take().is_some() {
                hide(out, RING_ID);
            }
            return;
        };
        let new_picture = !self
            .sent
            .as_ref()
            .is_some_and(|p| Arc::ptr_eq(p, &want.picture));
        if new_picture {
            if self.sent.is_some() {
                delete(out, MAP_ID);
            }
            send_png(out, MAP_ID, &want.picture.png);
            self.sent = Some(want.picture.clone());
            self.placed = None;
        }
        if self.placed != Some(want.placement) {
            place_map(out, &want.placement);
            self.placed = Some(want.placement);
        }
        if self.ring != want.ring {
            match &want.ring {
                Some(ring) => {
                    // Sending under the same id replaces the old ring and its placement.
                    send_rgba(out, RING_ID, ring.width, ring.height, &ring.rgba());
                    place_ring(out, ring);
                }
                None => hide(out, RING_ID),
            }
            self.ring = want.ring.clone();
        }
    }

    /// Brings reading mode's figures to `want`: new files are sent, moved ones re-placed, and
    /// those no longer wanted hidden.
    pub fn sync_figures(&mut self, want: &[Figure], out: &mut Vec<u8>) {
        for f in want {
            if self.figures_sent.get(&f.id) != Some(&f.path) {
                if self.figures_sent.contains_key(&f.id) {
                    delete(out, f.id);
                }
                send_png_file(out, f.id, &f.path);
                self.figures_sent.insert(f.id, f.path.clone());
                self.figures_placed.remove(&f.id);
            }
            if self.figures_placed.get(&f.id) != Some(f) {
                place_figure(out, f);
                self.figures_placed.insert(f.id, f.clone());
            }
        }
        let mut gone: Vec<u32> = self
            .figures_placed
            .keys()
            .filter(|id| !want.iter().any(|f| f.id == **id))
            .copied()
            .collect();
        gone.sort_unstable();
        for id in gone {
            hide(out, id);
            self.figures_placed.remove(&id);
        }
    }

    /// The terminal lost its images (the alternate screen was left and entered again): send
    /// everything afresh next time.
    pub fn forget(&mut self) {
        *self = Self::default();
    }

    /// Deletes both images, for leaving the UI.
    pub fn clear(&mut self, out: &mut Vec<u8>) {
        if self.sent.is_some() || self.ring.is_some() {
            delete(out, MAP_ID);
            delete(out, RING_ID);
        }
        let mut ids: Vec<u32> = self.figures_sent.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            delete(out, id);
        }
        self.forget();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_vectors() {
        let cases = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (plain, encoded) in cases {
            assert_eq!(base64(plain.as_bytes()), encoded);
        }
    }

    #[test]
    fn a_large_image_goes_in_chunks_with_keys_on_the_first() {
        let mut out = Vec::new();
        // 6000 bytes is 8000 base64 characters: two chunks.
        send_png(&mut out, 9, &[0u8; 6000]);
        let text = String::from_utf8(out).expect("ascii");
        let commands: Vec<&str> = text.split("\x1b\\").filter(|c| !c.is_empty()).collect();
        assert_eq!(commands.len(), 2);
        assert!(commands[0].starts_with("\x1b_Ga=t,f=100,t=d,i=9,q=2,m=1;"));
        assert!(commands[1].starts_with("\x1b_Gm=0;"));
        let payload: usize = commands
            .iter()
            .map(|c| c.split_once(';').map_or(0, |(_, p)| p.len()))
            .sum();
        assert_eq!(payload, 8000);
        assert!(
            commands
                .iter()
                .all(|c| c.split_once(';').map_or(0, |(_, p)| p.len()) <= CHUNK)
        );
    }

    fn picture() -> Arc<Picture> {
        Arc::new(Picture {
            png: vec![1, 2, 3],
            width: 100,
            height: 50,
            nodes: Vec::new(),
        })
    }

    fn placement(src_x: u32) -> Placement {
        Placement {
            src_x,
            src_y: 0,
            src_w: 50,
            src_h: 50,
            col: 2,
            row: 3,
            cols: 10,
            rows: 5,
        }
    }

    fn sync(screen: &mut Screen, want: Option<&Scene>) -> String {
        let mut out = Vec::new();
        screen.sync(want, &mut out);
        String::from_utf8(out).expect("ascii")
    }

    #[test]
    fn figures_are_sent_by_path_once_and_hidden_when_scrolled_away() {
        let mut screen = Screen::default();
        let fig = Figure {
            id: 7_400,
            path: "/r/fig.png".into(),
            col: 3,
            row: 4,
            cols: 20,
            rows: 6,
        };
        let mut out = Vec::new();
        screen.sync_figures(std::slice::from_ref(&fig), &mut out);
        let first = String::from_utf8(out).expect("ascii");
        // The payload is the path in base64; kitty reads the file.
        assert!(first.contains(&format!(
            "a=t,f=100,t=f,i=7400,q=2,m=0;{}",
            base64(b"/r/fig.png")
        )));
        assert!(first.contains("a=p,i=7400,p=1,c=20,r=6"));
        let mut out = Vec::new();
        screen.sync_figures(std::slice::from_ref(&fig), &mut out);
        assert!(out.is_empty(), "nothing changed");
        let moved = Figure {
            row: 2,
            ..fig.clone()
        };
        let mut out = Vec::new();
        screen.sync_figures(std::slice::from_ref(&moved), &mut out);
        let text = String::from_utf8(out).expect("ascii");
        assert!(!text.contains("a=t") && text.contains("a=p,i=7400"));
        let mut out = Vec::new();
        screen.sync_figures(&[], &mut out);
        assert!(
            String::from_utf8(out)
                .expect("ascii")
                .contains("a=d,d=i,i=7400")
        );
        let mut out = Vec::new();
        screen.clear(&mut out);
        assert!(
            String::from_utf8(out)
                .expect("ascii")
                .contains("a=d,d=I,i=7400")
        );
    }

    #[test]
    fn the_picture_is_sent_once_and_panning_only_moves_the_crop() {
        let mut screen = Screen::default();
        let pic = picture();
        let mut scene = Scene {
            picture: pic.clone(),
            placement: placement(0),
            ring: None,
        };
        let first = sync(&mut screen, Some(&scene));
        assert!(first.contains("a=t,f=100"), "{first:?}");
        assert!(
            first.contains("\x1b[4;3H"),
            "moves to row 4, column 3: {first:?}"
        );
        assert!(first.contains("a=p,i=7301,p=1,x=0,y=0,w=50,h=50,c=10,r=5"));
        assert_eq!(sync(&mut screen, Some(&scene)), "", "nothing changed");
        scene.placement = placement(25);
        let pan = sync(&mut screen, Some(&scene));
        assert!(!pan.contains("a=t"), "no resend: {pan:?}");
        assert!(pan.contains("x=25"));
        // Hidden while a popup is open, back after it without a resend.
        assert!(sync(&mut screen, None).contains("a=d,d=i,i=7301"));
        let back = sync(&mut screen, Some(&scene));
        assert!(!back.contains("a=t") && back.contains("a=p"));
        // A new picture replaces the old one's data.
        scene.picture = picture();
        let repaint = sync(&mut screen, Some(&scene));
        assert!(repaint.contains("a=d,d=I,i=7301") && repaint.contains("a=t,f=100"));
    }
}
