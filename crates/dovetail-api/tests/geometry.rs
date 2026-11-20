//! The canvas sizes the core places by are repeated in the page, which draws the frames
//! (`page/src/core/geometry.ts`; ts-rs exports no constants). This keeps the two copies equal.

use std::path::Path;

use dovetail_api::geometry::{CARD_H, CARD_W, GAP, HEADER, PAD};

#[test]
fn the_pages_canvas_sizes_are_the_cores() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../page/src/core/geometry.ts");
    let text = std::fs::read_to_string(&path).expect("geometry.ts");
    let value = |name: &str| -> i64 {
        let line = text
            .lines()
            .find(|l| l.starts_with(&format!("export const {name} = ")))
            .unwrap_or_else(|| panic!("{} has no {name}", path.display()));
        line.trim_end_matches(';')
            .rsplit(' ')
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("{name} is not a number in {line}"))
    };
    for (name, core) in [
        ("CARD_W", CARD_W),
        ("CARD_H", CARD_H),
        ("GAP", GAP),
        ("HEADER", HEADER),
        ("PAD", PAD),
    ] {
        assert_eq!(
            value(name),
            core,
            "{name} differs between the GUI and the core"
        );
    }
}
