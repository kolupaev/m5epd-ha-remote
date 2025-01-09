use std::marker::PhantomPinned;
use std::time::Duration;
use std::time::Instant;

use eg_seven_segment::{SevenSegmentStyle, SevenSegmentStyleBuilder};
use embedded_graphics::{pixelcolor::Gray4, prelude::*, primitives::Rectangle, text::Text};

use embedded_layout::layout::linear::FixedMargin;
use embedded_layout::layout::linear::LinearLayout;
use embedded_layout::{prelude::*, ViewGroup};
use u8g2_fonts::fonts;
use u8g2_fonts::U8g2TextStyle;
use uom::si::quantities::ThermodynamicTemperature;
use uom::si::{electric_potential::volt, thermodynamic_temperature::degree_fahrenheit};

use crate::state::AppState;
use crate::table::DisplayTable;
use crate::util::RectExt2;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error<DisplayError> {
    Generic,
    DisplayError(#[from] DisplayError),
    FontError(#[from] u8g2_fonts::Error<DisplayError>),
}

pub struct Renderer {
    segment_renderer: eg_seven_segment::SevenSegmentStyle<Gray4>,
    table: DisplayTable<(AppState, Duration), Gray4>,
    segmented_displays: Vec<Widget>,

    render_time: Duration,
    full_render: bool,
}

pub enum DrawResult {
    Complete(Rectangle),
    Partial(Rectangle),
    None,
}

#[derive(ViewGroup)]
pub enum WidgetType<'a> {
    LabeledSevenSeg(
        Text<'a, U8g2TextStyle<Gray4>>,
        Text<'a, SevenSegmentStyle<Gray4>>,
    ),
}

impl<'a> WidgetType<'a> {
    fn draw<T: DrawTarget<Color = Gray4>>(
        &self,
        dynamic_only: bool,
        target: &mut T,
    ) -> Result<Option<Rectangle>, T::Error> {
        match self {
            WidgetType::LabeledSevenSeg(label, value) => {
                value.draw(target)?;
                let mut bb = Some(value.bounding_box());
                if !dynamic_only {
                    label.draw(target)?;
                    bb.merge_rect(label.bounding_box());
                }
                Ok(bb)
            }
        }
    }
}

enum WidgetDataSource {
    State(fn(&AppState) -> String),
}

pub struct Widget {
    pub w_type: WidgetType<'static>,
    value: String,
    state_source: WidgetDataSource,
    changed: bool,
    _marker: PhantomPinned,
}

impl Widget {
    fn refresh_from_state(&mut self, state: &AppState) {
        if let Some(new_value) = match self.state_source {
            WidgetDataSource::State(f) => Some(f(state)),
        } {
            let changed = new_value != self.value;

            if changed {
                self.changed = true;
                self.value = new_value;
                match &mut self.w_type {
                    WidgetType::LabeledSevenSeg(label, value) => {
                        log::info!(
                            "Widget change detected: {}, changed to {}",
                            label.text,
                            self.value
                        );

                        // Hell of self-referncial structures,
                        // One-day: fixme
                        value.text =
                            unsafe { std::mem::transmute::<&str, &str>(self.value.as_str()) };
                    }
                }
            }
        }
    }

    fn draw<T: DrawTarget<Color = Gray4>>(
        &mut self,
        dynamic_only: bool,
        target: &mut T,
    ) -> Result<Option<Rectangle>, T::Error> {
        if !dynamic_only || self.changed {
            self.changed = false;
            Ok(self.w_type.draw(dynamic_only, target)?)
        } else {
            Ok(None)
        }
    }
}

impl View for Widget {
    fn translate_impl(&mut self, by: Point) {
        self.w_type.translate_impl(by);
    }

    fn bounds(&self) -> Rectangle {
        self.w_type.bounds()
    }
}

impl Renderer {
    fn txt_seven_segment(&self, label: &'static str, source: WidgetDataSource) -> Widget {
        let label = Text::new(
            label,
            Point::zero(),
            U8g2TextStyle::new(fonts::u8g2_font_spleen16x32_mr, Gray4::BLACK),
        );
        let mut display = Text::new("--.-", Point::zero(), self.segment_renderer);

        display.align_to_mut(&label, horizontal::Left, vertical::TopToBottom);

        Widget {
            w_type: WidgetType::LabeledSevenSeg(label, display),
            state_source: source,
            changed: false,
            value: String::new(),
            _marker: PhantomPinned,
        }
    }

    pub fn new(bounding_box: &Rectangle) -> Renderer {
        let mut r = Renderer {
            render_time: Duration::ZERO,
            segment_renderer: SevenSegmentStyleBuilder::new()
                .digit_size(Size::new(35, 70))
                .digit_spacing(6) // 5px spacing between digits
                .segment_width(12) // 5px wide segments
                .segment_color(Gray4::BLACK) // active segments are green
                .inactive_segment_color(Gray4::WHITE)
                .build(),
            table: DisplayTable::new(Gray4::BLACK, Gray4::WHITE)
                .expect("unable to create DisplayTable"),
            segmented_displays: Vec::new(),
            full_render: true,
        };

        r.init_widgets();
        r.update_layout(bounding_box);
        r
    }

    fn init_widgets(&mut self) {
        let table = &mut self.table;

        table.add_item("Counter", |s| format!("{:<5}", s.0.loop_counter));
        table.add_item("Time", |s| {
            format!("{:<6} s", s.0.time_since_boot.as_secs())
        });
        table.add_item("Voltage", |s| {
            format!("{:<6.4} V", s.0.batt_voltage.get::<volt>())
        });
        table.add_item("SOC", |s| format!("{:<3.2}", s.0.state_of_charge));
        table.add_item("iSOC", |s| {
            format!("{:<3.2}", s.0.initial_state_of_charge.unwrap_or(0_f32))
        });
        table.add_item("SOC d-rate", |s| {
            format!(
                "{:<7.4} 1/hr",
                s.0.state_of_charge_change_rate.unwrap_or(0_f32)
            )
        });
        table.add_item("Temp", |s| {
            format!(
                "{:<3.1} °F",
                s.0.temp_sensor
                    .map_or(0.0_f32, |t| t.get::<degree_fahrenheit>())
            )
        });
        table.add_item("Setpoint", |s| {
            format!(
                "{:<3.1} °F",
                s.0.temp_setpoint
                    .map_or(0.0_f32, |t| t.get::<degree_fahrenheit>())
            )
        });
        table.add_item("Render time", |s| format!("{:<6} ms", s.1.as_millis()));
        table.add_item("Net", |s| format!("{:<6?}", s.0.network_status));
        table.add_item("Heap free", |s| {
            format!("{:<6} kb", s.0.free_heap_bytes / 1024)
        });

        self.segmented_displays.push(self.txt_seven_segment(
            "temp °F",
            WidgetDataSource::State(|s| temp_str(s.temp_sensor)),
        ));
        self.segmented_displays.push(self.txt_seven_segment(
            "setpoint °F",
            WidgetDataSource::State(|s| temp_str(s.temp_setpoint)),
        ));
    }

    fn update_layout(&mut self, bounding_box: &Rectangle) {
        let mut v: Vec<Views<'_, Widget>> = Vec::new();

        let mut remaining = self.segmented_displays.as_mut_slice();
        let spacing = FixedMargin(62);
        while remaining.len() > 1 {
            let (l, r) = remaining.split_at_mut(2);
            remaining = r;
            let _ = LinearLayout::horizontal(Views::new(l))
                .with_spacing(spacing)
                .arrange();

            v.push(Views::new(l));
        }
        v.push(Views::new(remaining));

        let segment_views = LinearLayout::vertical(Views::new(&mut v))
            .with_spacing(spacing)
            .arrange();

        let segment_views = segment_views.align_to(bounding_box, horizontal::Center, vertical::Top);

        let segment_views = segment_views.translate(Point::new(0, 62));

        LinearLayout::vertical(&mut self.table)
            .arrange()
            .align_to(
                &segment_views,
                horizontal::NoAlignment,
                vertical::TopToBottom,
            )
            .align_to(bounding_box, horizontal::Left, vertical::NoAlignment);

        (&mut self.table).translate(Point::new(0, 26));
    }

    pub fn draw<Display, DisplayError>(
        &mut self,
        state: &AppState,
        display: &mut Display,
    ) -> Result<DrawResult, Error<DisplayError>>
    where
        Display: DrawTarget<Color = Gray4, Error = DisplayError>,
    {
        let render_start = Instant::now();
        let mut bb = None;
        for e in &mut self.segmented_displays {
            e.refresh_from_state(state);
        }

        let refresh_time = render_start - Instant::now();

        self.table.update(&(state.clone(), self.render_time));

        for e in &mut self.segmented_displays {
            bb.merge(&e.draw(!self.full_render, display)?);
        }
        bb.merge(&self.table.draw(!self.full_render, display)?);

        self.render_time = Instant::now() - render_start;

        log::info!(
            "Refresh time {} ms, render time {} ms",
            refresh_time.as_millis(),
            self.render_time.as_millis()
        );

        self.full_render = false;
        Ok(bb.map(DrawResult::Complete).unwrap_or(DrawResult::None))
    }
}

fn temp_str(temp: Option<ThermodynamicTemperature<f32>>) -> String {
    temp.map(|t| format!("{:4.1}", t.get::<degree_fahrenheit>()))
        .unwrap_or("--.-".to_owned())
}
