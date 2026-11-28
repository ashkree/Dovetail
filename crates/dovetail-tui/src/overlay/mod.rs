//! Floating overlays: the generic fuzzy [`picker::Picker`], switcher/finder items, MCP/usage/model
//! menus, quick capture and the driver-switch confirm modal.

use ratatui::layout::Rect;

pub mod capture;
pub mod confirm;
pub mod mcp_menu;
pub mod model_select;
pub mod picker;
pub mod switcher;
pub mod usage_menu;

/// Centers a box of `width` × `height` inside `screen`, clamping to `screen`'s bounds.
pub(crate) fn centered_rect(screen: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(screen.width);
    let height = height.min(screen.height);
    Rect {
        x: screen.x + (screen.width - width) / 2,
        y: screen.y + (screen.height - height) / 2,
        width,
        height,
    }
}
