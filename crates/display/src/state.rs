use std::time::Duration;

// use esp_idf_svc::sys::EspError;
use uom::si::{
    electric_potential::volt,
    quantities::{ElectricPotential, ThermodynamicTemperature},
    temperature_interval,
    thermodynamic_temperature::degree_fahrenheit,
};

pub type Voltage = uom::si::f32::ElectricPotential;

#[derive(Clone, Debug)]
pub struct AppState {
    pub updated_counter: u32,
    pub loop_counter: u32,
    pub time_since_boot: Duration,
    pub batt_voltage: ElectricPotential<f32>,
    pub state_of_charge: f32,
    pub initial_state_of_charge: Option<f32>,
    pub state_of_charge_change_rate: Option<f32>,
    pub network_status: NetworkStatus,
    pub free_heap_bytes: u32,

    pub temp_sensor: Option<ThermodynamicTemperature<f32>>,
    pub temp_setpoint: Option<ThermodynamicTemperature<f32>>,
}

#[derive(Clone, Debug)]
pub enum NetworkStatus {
    Initializing,
    WifiConnected,
    MqttConnected,
    Error,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> AppState {
        AppState {
            updated_counter: 0,
            loop_counter: 0,
            time_since_boot: Duration::ZERO,
            batt_voltage: Voltage::new::<volt>(0.0),
            state_of_charge: 0_f32,
            initial_state_of_charge: None,
            state_of_charge_change_rate: None,
            network_status: NetworkStatus::Initializing,
            free_heap_bytes: 0,
            temp_sensor: None,
            temp_setpoint: Some(temp_f_to_uom(72_f32)),
        }
    }

    pub fn set_temp_sensor_f(&mut self, temp: f32) {
        self.temp_sensor = Some(temp_f_to_uom(temp));
    }

    pub fn set_temp_setpoint_f(&mut self, temp: f32) {
        self.temp_setpoint = Some(temp_f_to_uom(temp));
    }

    pub fn adjust_temp_setpoint_f(&mut self, temp: f32) {
        if let Some(t) = self.temp_setpoint.as_mut() {
            *t += uom::si::f32::TemperatureInterval::new::<temperature_interval::degree_fahrenheit>(
                temp,
            );
        };
    }

    pub fn refresh_updated_counter(&mut self) {
        self.updated_counter += 1;
    }

    pub fn is_new(&self, other: u32) -> bool {
        self.updated_counter > other
    }
}

fn temp_f_to_uom(temp: f32) -> ThermodynamicTemperature<f32> {
    uom::si::f32::ThermodynamicTemperature::new::<degree_fahrenheit>(temp)
}
