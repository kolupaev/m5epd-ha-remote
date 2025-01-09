use embedded_graphics::primitives::Rectangle;
use embedded_layout::prelude::RectExt;
use log::warn;

pub(crate) fn merge(r1: Option<Rectangle>, r2: Option<Rectangle>) -> Option<Rectangle> {
    match (r1, r2) {
        (Some(r1), Some(r2)) => Some(r1.enveloping(&r2)),
        (Some(r1), None) => Some(r1),
        (None, Some(r2)) => Some(r2),
        (None, None) => None,
    }
}

pub(crate) trait RectExt2 {
    fn merge(&mut self, other: &Option<Rectangle>) -> &Self;
    fn merge_rect(&mut self, other: Rectangle) -> &Self;
}

impl RectExt2 for Option<Rectangle> {
    fn merge(&mut self, other: &Option<Rectangle>) -> &Self {
        let merge_result = match (*self, other) {
            (Some(r1), Some(r2)) => Some(r1.enveloping(r2)),
            (None, Some(r2)) => Some(*r2),
            _ => None,
        };

        if let Some(r) = merge_result {
            self.replace(r);
        }

        self
    }

    fn merge_rect(&mut self, other: Rectangle) -> &Self {
        self.merge(&Some(other))
    }
}

pub(crate) fn log_font_err<DisplayError>(
    err: u8g2_fonts::Error<DisplayError>,
) -> Result<Option<Rectangle>, DisplayError> {
    match err {
        u8g2_fonts::Error::BackgroundColorNotSupported => {
            warn!("Background color not supported");
            Ok(None)
        }
        u8g2_fonts::Error::GlyphNotFound(g) => {
            warn!("Glyph not found {}", g);
            Ok(None)
        }
        u8g2_fonts::Error::DisplayError(e) => Err(e),
    }
}
