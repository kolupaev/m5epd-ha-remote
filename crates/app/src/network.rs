use core::str;
use std::num::NonZero;

use embassy_futures::select::{select, Either};
use embassy_sync::{
    blocking_mutex::raw::{CriticalSectionRawMutex, RawMutex},
    channel::{Channel, Receiver, Sender},
};
use esp_idf_svc::{
    eventloop::{EspSystemEventLoop, System},
    hal::modem::Modem,
    mqtt::client::{EspAsyncMqttClient, EspAsyncMqttConnection, MqttClientConfiguration, QoS},
    nvs::EspDefaultNvsPartition,
    sys::EspError,
    timer::{EspTimerService, Task},
    wifi::{AsyncWifi, ClientConfiguration, Configuration, EspWifi},
};

use esp_idf_svc::hal::{modem::WifiModemPeripheral, peripheral::Peripheral};

use log::{info, warn};
use uom::si::thermodynamic_temperature::degree_fahrenheit;

use crate::{
    state_container::{StateStoreExt, STATE_STORE},
    APP_CONFIG,
};
use display::state::{AppState, NetworkStatus};
use embassy_time::Timer;

#[derive(Copy, Clone, Debug)]
enum MqttEvent {
    Connected,
    Disconnected,
    ReceivedSensorData { data: f32 },
    ReceivedSetpointData { data: f32 },
}

unsafe impl Send for MqttEvent {}
unsafe impl Sync for MqttEvent {}

fn parse_f32(data: &[u8]) -> Option<f32> {
    str::from_utf8(data)
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
}

fn ha_mqtt_registration_payload() -> serde_json::Value {
    serde_json::json!({
        "dev": {
            "ids": "m5paper",
            "name": "m5paper Remote"
        },
        "o": {
            "name": "m5remote2mqtt",
        },
        "cmps": {
            "setpoint": {
                "p": "number",
                "device_class": "temperature",
                "unit_of_measurement": "°F",
                "min": 32,
                "max": 90,
                "step": 0.5,
                "state_topic": "m5premote/setpoint/state",
                "command_topic": "m5premote/setpoint/set",
                "unique_id": "setpoint_temp_f",
            }
        },
    })
}

struct MqttConnectionProxy<'ch, M: RawMutex, const N: usize> {
    sender: Sender<'ch, M, MqttEvent, N>,
    connection: EspAsyncMqttConnection,
}

impl<'ch, M: RawMutex, const N: usize> MqttConnectionProxy<'ch, M, N> {
    async fn connection_loop(&mut self) {
        loop {
            match self.connection.next().await {
                Ok(evt) => {
                    log::info!("MQTT Event {:?}", evt.payload());

                    match evt.payload() {
                        esp_idf_svc::mqtt::client::EventPayload::Connected(_) => {
                            self.sender.send(MqttEvent::Connected).await;
                        }
                        esp_idf_svc::mqtt::client::EventPayload::Disconnected => {
                            self.sender.send(MqttEvent::Disconnected).await;
                        }
                        esp_idf_svc::mqtt::client::EventPayload::Received {
                            id: _,
                            topic: Some(topic),
                            data,
                            details: _,
                        } => {
                            let value = parse_f32(data);
                            let msg = match (topic, value) {
                                (topic, Some(temp)) if topic == APP_CONFIG.mqtt_sensor_topic => {
                                    Some(MqttEvent::ReceivedSensorData { data: temp })
                                }
                                ("m5premote/setpoint/set", Some(temp)) => {
                                    Some(MqttEvent::ReceivedSetpointData { data: temp })
                                }
                                (_, _) => {
                                    log::warn!("Unable to process mqtt command");
                                    None
                                }
                            };

                            if let Some(msg) = msg {
                                self.sender.send(msg).await;
                            }
                        }
                        esp_idf_svc::mqtt::client::EventPayload::BeforeConnect => {}
                        esp_idf_svc::mqtt::client::EventPayload::Published(_) => {}
                        esp_idf_svc::mqtt::client::EventPayload::Subscribed(_) => {}
                        _ => {
                            log::warn!("Unknown MQTT Event {:?}", evt.payload());
                        }
                    }
                }
                Err(e) => log::warn!("Mqtt err {}", e),
            }
        }
    }
}

struct MqttHandler<'ch, M: RawMutex, const N: usize, const NW: usize> {
    mqtt_id: String,
    receiver: Receiver<'ch, M, MqttEvent, N>,
    state_receiver: embassy_sync::watch::Receiver<'ch, M, AppState, NW>,
    client: EspAsyncMqttClient,
}

impl<'ch, M: RawMutex, const N: usize, const NW: usize> MqttHandler<'ch, M, N, NW> {
    async fn handler_loop(&mut self) {
        loop {
            let evt = select(self.receiver.receive(), self.state_receiver.changed()).await;

            match evt {
                Either::First(msg) => {
                    let r = self.handle_mqtt_evt(&msg).await;
                    log::info!("Handled {msg:?} {r:?}");
                }
                Either::Second(state) => {
                    let r = self.publish_state(&state).await;
                    log::info!("Publishied state to MQTT {r:?}")
                }
            }
        }
    }

    async fn handle_mqtt_evt(&mut self, msg: &MqttEvent) -> Result<(), EspError> {
        // <discovery_prefix>/<component>/[<node_id>/]<object_id>/config
        let ha_config_topic = "homeassistant/device/m5premote/config";

        match msg {
            MqttEvent::Connected => {
                self.client
                    .subscribe(APP_CONFIG.mqtt_sensor_topic, QoS::AtLeastOnce)
                    .await?;
                self.client
                    .subscribe("m5premote/setpoint/set", QoS::AtLeastOnce)
                    .await?;

                self.client
                    .publish(
                        ha_config_topic,
                        QoS::AtLeastOnce,
                        true,
                        ha_mqtt_registration_payload().to_string().as_bytes(),
                    )
                    .await?;

                STATE_STORE
                    .update(|s| s.network_status = NetworkStatus::MqttConnected)
                    .await;

                Ok(())
            }
            MqttEvent::Disconnected => Err(EspError::from_non_zero(NonZero::new(1).unwrap())),
            MqttEvent::ReceivedSensorData { data } => {
                STATE_STORE.update(|s| s.set_temp_sensor_f(*data)).await;
                Ok(())
            }
            MqttEvent::ReceivedSetpointData { data } => {
                STATE_STORE.update(|s| s.set_temp_setpoint_f(*data)).await;
                Ok(())
            }
        }
    }

    async fn publish_state(&mut self, state: &AppState) -> Result<(), EspError> {
        let setpoint = state.temp_setpoint;
        info!("Publishing setpoint {setpoint:?} to state topic");
        if let Some(setpoint) = setpoint {
            let setpoint_str = format!("{:.1}", setpoint.get::<degree_fahrenheit>());
            self.client
                .publish(
                    "m5premote/setpoint/state",
                    QoS::AtMostOnce,
                    true,
                    setpoint_str.as_bytes(),
                )
                .await?;
        }

        Ok(())
    }
}

async fn mqtt_loop() -> Result<(), EspError> {
    let (client, connection) = mqtt_create("m5paper")?;

    let channel: Channel<CriticalSectionRawMutex, MqttEvent, 15> = Channel::new();
    let (mqtt_sender, mqtt_receiver) = (channel.sender(), channel.receiver());
    let state_watcher = STATE_STORE
        .get()
        .change_watch
        .receiver()
        .expect("Unable to allocate receiver");

    let mut conn_proxy = MqttConnectionProxy {
        sender: mqtt_sender,
        connection,
    };
    let mut handler_loop = MqttHandler {
        receiver: mqtt_receiver,
        state_receiver: state_watcher,
        client,
        mqtt_id: "m5premote".to_owned(),
    };

    let _r = select(conn_proxy.connection_loop(), handler_loop.handler_loop()).await;

    log::error!("Mqtt loop terminated");
    Ok(())
}

async fn wifi_start_and_block(wifi: &mut AsyncWifi<&mut EspWifi<'_>>) -> Result<(), EspError> {
    if !wifi.is_started()? {
        wifi.start().await?;
        info!("Wifi started");
    }

    if !wifi.is_connected()? {
        wifi.connect().await?;
        info!("Wifi connected");
    }

    if !wifi.is_up()? {
        wifi.wait_netif_up().await?;
        info!(
            "Wifi netif up: IP: {:?}",
            wifi.wifi().ap_netif().get_ip_info()?
        );
    }
    STATE_STORE
        .update(|s| s.network_status = NetworkStatus::WifiConnected)
        .await;

    wifi.wifi_wait(|m| m.is_up(), None).await?;

    Ok(())
}

pub async fn network_loop(
    sys_loop: &esp_idf_svc::eventloop::EspEventLoop<System>,
    timer_service: &EspTimerService<Task>,
    nvs: &esp_idf_svc::nvs::EspNvsPartition<esp_idf_svc::nvs::NvsDefault>,
    modem: Modem,
) -> Result<(), EspError> {
    let mut esp_wifi = wifi_create(sys_loop, nvs, modem).await?;
    let mut wifi = AsyncWifi::wrap(&mut esp_wifi, sys_loop.clone(), timer_service.clone())?;

    let _ = select(
        async move {
            loop {
                let r = wifi_start_and_block(&mut wifi).await;
                warn!("Wifi connnection terminated {r:?}, restarting in 15s");

                STATE_STORE
                    .update(|s| s.network_status = NetworkStatus::Error)
                    .await;

                let _ = wifi.stop().await;
                Timer::after_secs(15).await;
            }
        },
        mqtt_loop(),
    )
    .await;

    log::error!("Network loop terminated");
    Ok(())

    // https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/network/esp_wifi.html#_CPPv414wifi_ps_type_t
}

async fn wifi_create<M: WifiModemPeripheral>(
    sys_loop: &EspSystemEventLoop,
    nvs: &EspDefaultNvsPartition,
    modem: impl Peripheral<P = M> + 'static,
) -> Result<EspWifi<'static>, EspError> {
    let mut esp_wifi = EspWifi::new(modem, sys_loop.clone(), Some(nvs.clone()))?;

    esp_wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: APP_CONFIG.wifi_ssid.try_into().unwrap(),
        password: APP_CONFIG.wifi_psk.try_into().unwrap(),
        ..Default::default()
    }))?;

    esp_idf_svc::hal::sys::esp!(unsafe {
        esp_idf_svc::sys::esp_wifi_set_ps(esp_idf_svc::sys::wifi_ps_type_t_WIFI_PS_MIN_MODEM)
    })?;

    info!("Wifi created");
    Ok(esp_wifi)
}

fn mqtt_create(client_id: &str) -> Result<(EspAsyncMqttClient, EspAsyncMqttConnection), EspError> {
    let (mqtt_client, mqtt_conn) = EspAsyncMqttClient::new(
        APP_CONFIG.mqtt_server,
        &MqttClientConfiguration {
            client_id: Some(client_id),
            ..Default::default()
        },
    )?;

    info!("MQTT client created");
    Ok((mqtt_client, mqtt_conn))
}
