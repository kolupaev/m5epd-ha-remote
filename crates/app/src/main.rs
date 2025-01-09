#![feature(async_closure)]
mod hardware;
mod network;
mod state_container;
mod ui;

use core::str;
use std::{num::NonZeroU32, thread, time::Instant};

use embassy_futures::select::select3;
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::{
        delay::FreeRtos,
        gpio::{AnyInputPin, InterruptType, Level, Pin},
        task::notification::Notification,
    },
    nvs::EspDefaultNvsPartition,
    sys::EspError,
    timer::EspTimerService,
};

use esp_idf_svc::hal::gpio::PinDriver;

use embassy_time::Timer;
use hardware::*;
use network::network_loop;
use simple_moving_average::{SumTreeSMA, SMA};
use state_container::{StateStoreExt, STATE_STORE};
use ui::display_loop;
use uom::si::electric_potential::volt;

/// This configuration is picked up at compile time by `build.rs` from the
/// file `cfg.toml`.
/// Defines CONFIG
#[derive(Debug)]
#[toml_cfg::toml_config]
pub struct Config {
    #[default("")]
    wifi_ssid: &'static str,
    #[default("")]
    wifi_psk: &'static str,
    #[default("")]
    mqtt_server: &'static str,
    #[default("")]
    mqtt_sensor_topic: &'static str,
}

pub static APP_CONFIG: Config = CONFIG;

fn main() -> Result<(), EspError> {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    let sys_loop = EspSystemEventLoop::take().unwrap();
    let timer_service = EspTimerService::new().unwrap();
    let nvs = EspDefaultNvsPartition::take().unwrap();
    let SystemPerepherials {
        power:
            Power {
                main: mut pw_main,
                external: mut _pw_ext,
                display: mut pw_display,
            },
        modem,
        display: mut display_hw,
        batt_adc: mut adc,
        batt_adc_pin: mut adc_pin,
        buttons,
    } = SystemPerepherials::take();

    // https://docs.rs/data-encoding/latest/data_encoding/
    // let mut mac = [0u8; 8];
    // esp_idf_svc::sys::esp!(unsafe { esp_idf_svc::sys::esp_efuse_mac_get_default(mac.as_mut_ptr()) })?; //

    pw_main.set_high()?;
    // power.external.set_high();
    pw_display.set_high()?;

    // does not wake up
    let pm_config = esp_idf_svc::sys::esp_pm_config_esp32_t {
        max_freq_mhz: 80,
        min_freq_mhz: 40,
        light_sleep_enable: true,
    };

    //https://docs.espressif.com/projects/esp-idf/en/release-v4.4/esp32/api-reference/system/sleep_modes.html

    esp_idf_svc::sys::esp!(unsafe { esp_idf_svc::sys::gpio_sleep_sel_dis(pw_main.pin()) })?;
    esp_idf_svc::sys::esp!(unsafe { esp_idf_svc::sys::gpio_sleep_sel_dis(pw_display.pin()) })?;
    esp_idf_svc::sys::esp!(unsafe { esp_idf_svc::sys::esp_sleep_enable_gpio_wakeup() })?;
    esp_idf_svc::sys::esp!(unsafe {
        esp_idf_svc::sys::gpio_wakeup_enable(
            buttons.down.pin(),
            esp_idf_svc::sys::gpio_int_type_t_GPIO_INTR_LOW_LEVEL,
        )
    })?;
    esp_idf_svc::sys::esp!(unsafe {
        esp_idf_svc::sys::gpio_wakeup_enable(
            buttons.up.pin(),
            esp_idf_svc::sys::gpio_int_type_t_GPIO_INTR_LOW_LEVEL,
        )
    })?;
    esp_idf_svc::sys::esp!(unsafe {
        esp_idf_svc::sys::gpio_wakeup_enable(
            buttons.push.pin(),
            esp_idf_svc::sys::gpio_int_type_t_GPIO_INTR_LOW_LEVEL,
        )
    })?;

    esp_idf_svc::sys::esp!(unsafe {
        esp_idf_svc::sys::esp_pm_configure(&pm_config as *const _ as *const _)
    })?;
    log::info!("Sleep configured");

    log::info!("Configuration: {:?}", APP_CONFIG);

    {
        thread::spawn(move || {
            let r = button_thread(buttons);
            log::error!("Button thread failed with {r:?}");
        });
    }

    let mut batt_sensor = batt_sensor_create(&mut adc, &mut adc_pin)?;

    esp_idf_svc::hal::task::block_on(async {
        log::info!("Initialization complete");

        let result = select3(
            network_loop(&sys_loop, &timer_service.clone(), &nvs, modem),
            async move {
                Timer::after_millis(100).await;
                let display = display_create(&mut display_hw)?;
                display_loop(display).await?;
                Ok::<(), EspError>(())
            },
            update_loop(&mut batt_sensor),
        )
        .await;

        log::error!("main loop exited {:?}", result);
        Ok::<(), EspError>(())
    })?;

    drop(pw_display);
    drop(pw_main);
    Result::Ok(())
}

struct ButtonsHandler<'a, const SZ: usize> {
    buttons: [PinDriver<'a, AnyInputPin, esp_idf_svc::hal::gpio::Input>; SZ],
    notification: Notification,
}

impl<'a, const SZ: usize> ButtonsHandler<'a, SZ> {
    fn new(buttons: [AnyInputPin; SZ]) -> Result<Self, EspError> {
        let mut drivers =
            buttons.map(|p| PinDriver::input(p).expect("Unable to create pin driver"));

        let notification = Notification::new();
        let notifier = notification.notifier();

        for (i, b) in drivers.iter_mut().enumerate() {
            b.set_interrupt_type(InterruptType::LowLevel)?;
            let n = notifier.clone();
            unsafe {
                b.subscribe(move || {
                    n.notify_and_yield(NonZeroU32::new((i + 1) as u32).unwrap());
                })?;
            }
        }

        Ok(Self {
            buttons: drivers,
            notification,
        })
    }

    fn enable_interrupts(&mut self) -> Result<(), EspError> {
        for b in self.buttons.iter_mut() {
            b.enable_interrupt()?;
        }
        Ok(())
    }

    fn wait(&mut self) -> Option<usize> {
        self.notification.wait(esp_idf_svc::hal::delay::BLOCK);
        for (i, b) in self.buttons.iter_mut().enumerate() {
            if b.get_level() == Level::Low {
                return Some(i);
            }
        }
        None
    }
}

fn button_thread(buttons: Buttons) -> Result<(), EspError> {
    let mut handler = ButtonsHandler::new([buttons.up, buttons.push, buttons.down])?;

    handler.enable_interrupts()?;
    loop {
        if let Some(button) = handler.wait() {
            log::info!("Button pressed {button}",);
            let new_state = {
                let mut w = esp_idf_svc::hal::task::block_on(async {
                    STATE_STORE.get().state.write().await
                });
                let adjustment = match button {
                    0 => 0.5_f32,
                    2 => -0.5_f32,
                    _ => 0.0_f32,
                };
                w.adjust_temp_setpoint_f(adjustment);
                w.refresh_updated_counter();
                w.clone()
            };
            STATE_STORE.get().change_watch.sender().send(new_state);
            FreeRtos::delay_ms(500);
        }
        handler.enable_interrupts()?;
    }

    // unreachable!("Button thread exited");
}

async fn update_loop(batt_sensor: &mut BatteryVoltageSensor<'_>) -> Result<(), EspError> {
    const LOOP_TICK_S: u32 = 30;
    const UPDATE_IVL_S: u32 = 15 * 60;

    const UPDATE_TICKS: u32 = UPDATE_IVL_S / LOOP_TICK_S;
    const BATT_MEASURE_TICKS: usize = (60 / LOOP_TICK_S) as usize;

    let loop_start = Instant::now();
    let mut initial_soc = None;
    let mut loop_counter: u32 = 0;
    let mut batt_voltage_sma = SumTreeSMA::<f32, f32, BATT_MEASURE_TICKS>::new();

    log::info!("Update loop initialized");

    loop {
        loop_counter += 1;

        let time_since_boot = Instant::now() - loop_start;

        let voltage = batt_sensor.read()?;
        batt_voltage_sma.add_sample(voltage.get::<volt>());
        let voltage_avg = Voltage::new::<volt>(batt_voltage_sma.get_average());

        if loop_counter == (BATT_MEASURE_TICKS as u32) * 3 {
            initial_soc = Some(BatteryVoltageSensor::soc(voltage_avg));
        }

        let soc_change_rate = initial_soc.map(|isoc| {
            (isoc - BatteryVoltageSensor::soc(voltage_avg)) * 1000_f32 * 3600_f32
                / (time_since_boot.as_millis() as f32)
        });

        let heap_free = unsafe { esp_idf_svc::sys::esp_get_free_heap_size() };

        let should_trigger_update = loop_counter == 1 || loop_counter % UPDATE_TICKS == 0;

        STATE_STORE
            .update_and_trigger(should_trigger_update, |writer| {
                writer.loop_counter = loop_counter;
                writer.time_since_boot = time_since_boot;
                writer.batt_voltage = voltage_avg;
                writer.state_of_charge = BatteryVoltageSensor::soc(voltage_avg);
                writer.state_of_charge_change_rate = soc_change_rate;
                writer.initial_state_of_charge = initial_soc;
                writer.free_heap_bytes = heap_free;

                log::info!("Update loop state: {:?}", writer);
            })
            .await;

        Timer::after_secs(LOOP_TICK_S.into()).await;
    }
    // unreachable!("update_loop exited");
}
