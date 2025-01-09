use std::{convert::Infallible, time::Duration};

use display::{
    renderer::{DrawResult, Error, Renderer},
    state::{AppState, Voltage},
};
use embedded_graphics::{pixelcolor::Gray4, prelude::*};
use embedded_graphics_simulator::{OutputSettingsBuilder, SimulatorDisplay, Window};
use uom::si::electric_potential::volt;

fn main() -> Result<(), Error<Infallible>> {
    let mut display = SimulatorDisplay::<Gray4>::new(Size::new(540, 960));

    let mut renderer = Renderer::new(&display.bounding_box());

    let mut state = AppState {
        updated_counter: 2460,
        loop_counter: 2460,
        time_since_boot: Duration::from_secs(73849),
        batt_voltage: Voltage::new::<volt>(4.0141_f32),
        state_of_charge: 0.79,
        initial_state_of_charge: Some(0.98),
        state_of_charge_change_rate: Some(0.0088),
        network_status: display::state::NetworkStatus::MqttConnected,
        free_heap_bytes: 189000,
        temp_sensor: None,
        temp_setpoint: None,
    };

    state.set_temp_sensor_f(73.2_f32);
    state.set_temp_setpoint_f(72.5_f32);

    display.clear(Gray4::WHITE)?;

    let mut counter = 0;
    loop {
        if let DrawResult::Partial(_) = renderer.draw(&state, &mut display)? {
            counter += 1;
            log::info!("Draw counter: {}", counter);
            if counter < 10 {
                continue;
            }
        }

        break;
    }

    let output_settings = OutputSettingsBuilder::new()
        .pixel_spacing(0)
        .scale(1)
        .build();
    Window::new("Display screen", &output_settings).show_static(&display);

    Ok(())
}
