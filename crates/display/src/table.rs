use std::cmp::max;

use embedded_graphics::{
    prelude::{DrawTarget, PixelColor, Point, Size},
    primitives::Rectangle,
};
use u8g2_fonts::{fonts, types::FontColor, FontRenderer, LookupError};

use crate::util::{log_font_err, merge, RectExt2};

pub(crate) struct DisplayTable<T, Color>
where
    Color: PixelColor,
{
    font: FontRenderer,
    text_offset: Point,
    item_bounding_box: Rectangle,
    pub items: Vec<DisplayTableItem<T>>,
    color_fg: Color,
    color_bg: Color,
}

pub(crate) struct DisplayTableItem<T> {
    name: &'static str,
    last_value: Option<String>,
    changed: bool,
    pub bounds: Rectangle,
    value_fn: fn(&T) -> String,
}

impl<T, C> DisplayTable<T, C>
where
    C: PixelColor,
{
    pub fn new(fg: C, bg: C) -> Result<DisplayTable<T, C>, LookupError> {
        let font =
            FontRenderer::new::<fonts::u8g2_font_spleen16x32_mr>().with_ignore_unknown_chars(true);

        let title_bb = font
            .get_rendered_dimensions(
                "SOC d-rate: ",
                Point::zero(),
                u8g2_fonts::types::VerticalPosition::Top,
            )?
            .bounding_box
            .expect("Failed to get rendered dimensions");

        let text_bb = font
            .get_rendered_dimensions(
                "0.0000 1/hr",
                Point::zero(),
                u8g2_fonts::types::VerticalPosition::Top,
            )?
            .bounding_box
            .expect("Failed to get rendered dimensions");

        Ok(DisplayTable {
            font,
            item_bounding_box: Rectangle::new(
                Point::zero(),
                Size::new(
                    title_bb.size.width + text_bb.size.width,
                    max(title_bb.size.height, text_bb.size.height),
                ),
            ),
            text_offset: Point::new(title_bb.size.width as i32, 0),
            items: vec![],
            color_fg: fg,
            color_bg: bg,
        })
    }

    pub fn add_item(&mut self, name: &'static str, value_fn: fn(&T) -> String) {
        let i = DisplayTableItem {
            name,
            last_value: None,
            bounds: self.item_bounding_box,
            value_fn,
            changed: false,
        };
        self.items.push(i);
    }

    pub fn draw<D>(
        &mut self,
        dynamic_only: bool,
        target: &mut D,
    ) -> Result<Option<Rectangle>, D::Error>
    where
        D: DrawTarget<Color = C>,
    {
        let mut bb = None;

        if !dynamic_only {
            log::info!("Rendering labels");
            bb.merge(&self.draw_labels(target).or_else(log_font_err)?);
        }
        bb.merge(
            &self
                .draw_values(dynamic_only, target)
                .or_else(log_font_err)?,
        );
        Ok(bb)
    }

    pub fn draw_labels<Display, DisplayError>(
        &self,
        display: &mut Display,
    ) -> Result<Option<Rectangle>, u8g2_fonts::Error<Display::Error>>
    where
        Display: DrawTarget<Color = C, Error = DisplayError>,
    {
        let mut bb: Option<Rectangle> = None;
        let font = &self.font;
        for i in &self.items {
            let text_box = font.render_aligned(
                i.name,
                i.bounds.top_left,
                u8g2_fonts::types::VerticalPosition::Top,
                u8g2_fonts::types::HorizontalAlignment::Left,
                FontColor::WithBackground {
                    fg: self.color_fg,
                    bg: self.color_bg,
                },
                display,
            )?;

            bb.merge(&text_box);
        }

        Ok(bb)
    }

    pub fn draw_values<Display, DisplayError>(
        &mut self,
        dynamic_only: bool,
        display: &mut Display,
    ) -> Result<Option<Rectangle>, u8g2_fonts::Error<Display::Error>>
    where
        Display: DrawTarget<Color = C, Error = DisplayError>,
    {
        let mut bb: Option<Rectangle> = None;
        let font = &self.font;
        for i in &mut self.items {
            if !dynamic_only || i.changed {
                if let Some(v) = i.last_value.as_ref() {
                    let text_box = font.render_aligned(
                        v.as_str(),
                        Point::new(
                            i.bounds.top_left.x + self.text_offset.x,
                            i.bounds.top_left.y + self.text_offset.y,
                        ),
                        u8g2_fonts::types::VerticalPosition::Top,
                        u8g2_fonts::types::HorizontalAlignment::Left,
                        FontColor::WithBackground {
                            fg: self.color_fg,
                            bg: self.color_bg,
                        },
                        display,
                    )?;
                    i.changed = false;
                    bb = merge(bb, text_box);
                }
            }
        }

        Ok(bb)
    }

    pub fn update(&mut self, state: &T) {
        for i in &mut self.items {
            let new_value = (i.value_fn)(state);
            if i.last_value.as_ref() != Some(&new_value) {
                i.last_value = Some(new_value);
                i.changed = true;
                log::info!("Change detected in {}", i.name);
            }
        }
    }
}
