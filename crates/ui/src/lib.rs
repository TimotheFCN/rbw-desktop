//! The rbw-desktop design system: theme tokens for the macOS and GNOME looks,
//! motion, window chrome and the components the app is built from.
//!
//! Components are plain GPUI elements styled from [`Theme`]. Behaviour
//! (activation, focus, text editing, accessibility roles) comes from
//! gpui-base's unstyled controls; everything visible is defined here.

mod button;
mod card;
pub mod chrome;
mod controls;
mod dialog;
mod fields;
mod glass;
mod icon;
mod menu;
pub mod motion;
mod platform;
mod rows;
mod sheet;
mod theme;
mod toast;

pub use button::{Button, ButtonStyle, button};
pub use card::{Card, FieldRow, card, copied_badge, field_row, form_row, toggle_row};
pub use controls::{
    SegmentedControl, Slider, Switch, progress_ring, segmented_control, slider, spinner, switch,
};
pub use dialog::{ConfirmDialog, ConfirmEvent, Confirmation};
pub use fields::{SearchField, TextField, field_error, password_text, search_field, text_field};
pub use glass::{Glass, Material, glass};
pub use icon::{Assets, Icon, IconName, icon};
pub use menu::{MenuItem, Opening, Popover, menu_item, menu_separator, popover};
pub use rows::{ListRow, NavRow, Tile, app_icon, avatar, list_row, nav_row, section_label, tile};
pub use sheet::{SheetSlot, surface};
pub use theme::{ActiveTheme, Appearance, Colors, Look, Metrics, Theme};
pub use toast::ToastSlot;

use gpui::App;

/// Initializes gpui-base and the theme. Call once, before opening windows.
pub fn init(cx: &mut App) {
    gpui::base::init(cx);
    #[cfg(not(target_os = "macos"))]
    platform::load_fonts(cx);
    Theme::init(cx);
    sync_base_theme(cx);
    dialog::init(cx);
    menu::init(cx);
}

/// gpui-base's controls (text fields, focus rings) read their colours from its
/// own theme; keep that in step with ours.
fn sync_base_theme(cx: &mut App) {
    let theme = Theme::global(cx).clone();
    let base = gpui::base::Theme::global_mut(cx);
    let colors = &mut base.tokens.colors;
    colors.foreground = theme.colors.text;
    colors.muted_foreground = theme.colors.text_tertiary;
    colors.accent = theme.colors.accent;
    colors.ring = theme.colors.focus_ring;
    colors.selection = theme.colors.accent.opacity(0.3);
    colors.background = theme.colors.window;
    colors.surface = gpui::transparent_black();
    colors.border = theme.colors.separator;
}
