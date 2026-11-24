//! The app's line icons, drawn on a 16-point grid and tinted with the text
//! colour of wherever they are placed.

use std::borrow::Cow;

use std::panic::Location;

use gpui::{
    App, AssetSource, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, LayoutId,
    Pixels, Result, SharedString, StyleRefinement, Styled, Svg, Window, px, svg,
};

macro_rules! icons {
    ($($variant:ident => $file:literal,)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum IconName {
            $($variant,)*
        }

        impl IconName {
            const ALL: &[IconName] = &[$(Self::$variant,)*];

            fn path(self) -> &'static str {
                match self {
                    $(Self::$variant => concat!("icons/", $file, ".svg"),)*
                }
            }
        }

        fn icon_bytes(path: &str) -> Option<&'static [u8]> {
            match path {
                $(concat!("icons/", $file, ".svg") => {
                    Some(include_bytes!(concat!("../assets/icons/", $file, ".svg")))
                })*
                _ => None,
            }
        }
    };
}

icons! {
    ArrowRight => "arrow",
    Back => "back",
    Badge => "badge",
    Calendar => "calendar",
    Card => "card",
    Check => "check",
    ChevronRight => "chev",
    ChevronsUpDown => "updown",
    Clock => "clock",
    Close => "x",
    Copy => "copy",
    Dice => "dice",
    Down => "down",
    Edit => "pen",
    External => "ext",
    Eye => "eye",
    Filter => "filter",
    Folder => "folder",
    Gear => "gear",
    Globe => "globe",
    Grid => "grid",
    Identity => "user",
    Info => "info",
    Key => "key",
    Lock => "lock",
    Mail => "mail",
    Maximize => "maximize",
    Menu => "menu",
    Minimize => "minimize",
    Minus => "minus",
    Note => "note",
    Phone => "phone",
    Pin => "pin",
    Plus => "plus",
    Refresh => "sync",
    Search => "search",
    Shield => "shield",
    Sliders => "sliders",
    Star => "star",
    Terminal => "term",
    Text => "text",
    TouchId => "finger",
    Warning => "warning",
    Window => "window",
}

/// Serves the icons to GPUI's `svg()` element.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(icon_bytes(path).map(Cow::Borrowed))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(IconName::ALL
            .iter()
            .map(|icon| icon.path())
            .filter(|p| p.starts_with(path))
            .map(SharedString::from)
            .collect())
    }
}

/// An icon. Without a colour of its own it takes the text colour of where it
/// is placed, which GPUI's `svg` does not do by itself.
pub struct Icon {
    svg: Svg,
}

impl Icon {
    pub fn new(name: IconName) -> Self {
        Self {
            svg: svg().path(name.path()).size(px(15.)).flex_none(),
        }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.svg = self.svg.size(size);
        self
    }
}

pub fn icon(name: IconName) -> Icon {
    Icon::new(name)
}

impl Styled for Icon {
    fn style(&mut self) -> &mut StyleRefinement {
        self.svg.style()
    }
}

impl Element for Icon {
    type RequestLayoutState = <Svg as Element>::RequestLayoutState;
    type PrepaintState = <Svg as Element>::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.svg.id()
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        self.svg.source_location()
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.svg.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.svg
            .prepaint(id, inspector_id, bounds, layout, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let text = &mut self.svg.style().text;
        if text.color.is_none() {
            text.color = Some(window.text_style().color);
        }
        self.svg
            .paint(id, inspector_id, bounds, layout, prepaint, window, cx);
    }
}
