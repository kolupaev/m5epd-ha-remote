use embedded_graphics::pixelcolor::Gray4;
use esp_idf_svc::sys::EspError;

use embedded_graphics::prelude::*;
use it8951::*;
use log::info;

use display::renderer::{DrawResult, Renderer};

use crate::{hardware::M5Display, state_container::STATE_STORE};

pub async fn display_loop(display: M5Display<'_>) -> Result<(), EspError> {
    let state = STATE_STORE.get();
    let mut display = display;
    let display_bb = display.bounding_box();

    display.clear(Gray4::WHITE).expect("clear");
    display.display(WaveformMode::Init).expect("display update");

    let mut renderer = Renderer::new(&display.bounding_box());
    let mut watcher = state
        .change_watch
        .receiver()
        .expect("Unable to allocate state watcher");
    loop {
        let app_state = { state.state.read().await.clone() };
        let result = renderer.draw(&app_state, &mut display).expect("Draw error");
        let last_updated_counter = app_state.updated_counter;

        let (area_to_refresh, sleep) = match result {
            DrawResult::Partial(bb) => {
                log::info!("Partial draw {bb:?}");
                (Some(bb), false)
            }
            DrawResult::Complete(bb) => {
                log::info!("Complete draw {bb:?}");
                (Some(bb), true)
            }
            DrawResult::None => {
                log::info!("No screen updates");
                (None, true)
            }
        };

        if let Some(bb) = area_to_refresh {
            let bb = display_bb.intersection(&bb);

            info!("Refreshing {bb:?}");
            display
                .display_area(
                    &AreaImgInfo {
                        area_x: bb.top_left.x as u16,
                        area_y: bb.top_left.y as u16,
                        area_w: bb.size.width as u16,
                        area_h: bb.size.height as u16,
                    },
                    WaveformMode::A2, // WaveformMode::GLR16,
                )
                .expect("display update");
        }

        if sleep && !state.state.read().await.is_new(last_updated_counter) {
            display = {
                let display = display.sleep().expect("sleep");
                info!("Screen powered down, awaiting change");
                watcher.changed().await;
                display.sys_run().expect("wake")
            };
            info!("Display awakened");
        } else {
            info!("One more refresh cycle, sleep: {sleep}");
        }
    }
    // unreachable!("display_loop exited");
}
