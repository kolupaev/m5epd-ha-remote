use average::{Estimate, MeanWithError};
use dummy_pin::DummyPin;
use esp_idf_svc::hal::{
    adc::*,
    delay::Delay,
    gpio::{AnyInputPin, AnyOutputPin, Gpio35, Output, PinDriver},
    modem::Modem,
    prelude::Peripherals,
    spi::{SpiDeviceDriver, SpiDriverConfig, SPI3},
    sys::EspError,
    units::MegaHertz,
};
use interp::InterpMode;
use it8951::{
    interface::{IT8951Interface, IT8951SPIInterface},
    memory_converter_settings::MemoryConverterSetting,
    Config, Run, IT8951,
};
use oneshot::{AdcChannelDriver, AdcDriver};
use uom::si::electric_potential::{millivolt, volt};

pub type M5Display<'a> = IT8951<
    IT8951SPIInterface<
        SpiDeviceDriver<'a, esp_idf_svc::hal::spi::SpiDriver<'a>>,
        PinDriver<'a, AnyInputPin, esp_idf_svc::hal::gpio::Input>,
        DummyPin<dummy_pin::level::High>,
        Delay,
    >,
    Run,
>;

pub struct SystemPerepherials<'a> {
    pub modem: Modem,
    pub display: DisplayPerepherials,
    pub power: Power<'a>,
    pub batt_adc: ADC1,
    pub batt_adc_pin: Gpio35,
    pub buttons: Buttons,
}

pub struct DisplayPerepherials {
    pub spi: SPI3,
    pub cs: AnyOutputPin,
    pub mi: AnyInputPin,
    pub mo: AnyOutputPin,
    pub sck: AnyOutputPin,
    pub busy: AnyInputPin,
}

pub struct Buttons {
    pub up: AnyInputPin,
    pub down: AnyInputPin,
    pub push: AnyInputPin,
}

pub struct Power<'a> {
    pub main: PinDriver<'a, AnyOutputPin, Output>,
    pub external: PinDriver<'a, AnyOutputPin, Output>,
    pub display: PinDriver<'a, AnyOutputPin, Output>,
}

pub struct BatteryVoltageSensor<'a> {
    adc_channel: AdcChannelDriver<'a, Gpio35, Box<AdcDriver<'a, ADC1>>>,
}

pub type Voltage = uom::si::f32::ElectricPotential;

impl SystemPerepherials<'_> {
    pub fn take() -> Self {
        let peripherals = Peripherals::take().expect("unable to get peripherals");

        SystemPerepherials {
            modem: peripherals.modem,
            batt_adc: peripherals.adc1,
            batt_adc_pin: peripherals.pins.gpio35,
            power: Power {
                main: PinDriver::output(AnyOutputPin::from(peripherals.pins.gpio2))
                    .expect("main power pin"),
                external: PinDriver::output(AnyOutputPin::from(peripherals.pins.gpio5))
                    .expect("ext power pin"),
                display: PinDriver::output(AnyOutputPin::from(peripherals.pins.gpio23))
                    .expect("display power pin"),
            },

            display: DisplayPerepherials {
                spi: peripherals.spi3,
                cs: peripherals.pins.gpio15.into(),
                mi: peripherals.pins.gpio13.into(),
                mo: peripherals.pins.gpio12.into(),
                sck: peripherals.pins.gpio14.into(),
                busy: peripherals.pins.gpio27.into(),
            },

            buttons: Buttons {
                up: peripherals.pins.gpio37.into(),
                push: peripherals.pins.gpio38.into(),
                down: peripherals.pins.gpio39.into(),
            },
        }
    }
}

pub fn batt_sensor_create<'a>(
    adc: &'a mut ADC1,
    batt_pin: &'a mut Gpio35,
) -> Result<BatteryVoltageSensor<'a>, EspError> {
    BatteryVoltageSensor::new(adc, batt_pin)
}

impl<'a> BatteryVoltageSensor<'a> {
    pub fn new(adc: &'a mut ADC1, batt_pin: &'a mut Gpio35) -> Result<Self, EspError> {
        use esp_idf_svc::hal::adc::attenuation::DB_11;
        use esp_idf_svc::hal::adc::oneshot::config::AdcChannelConfig;
        use esp_idf_svc::hal::adc::oneshot::AdcDriver;
        use esp_idf_svc::hal::adc::oneshot::*;

        let adc: AdcDriver<'a, ADC1> = AdcDriver::new(adc)?;
        let config = AdcChannelConfig {
            attenuation: DB_11,
            calibration: config::Calibration::Line,
            resolution: Resolution::Resolution12Bit,
        };

        let adc = Box::new(adc);
        let adc_pin: AdcChannelDriver<'_, Gpio35, Box<AdcDriver<'a, ADC1>>> =
            AdcChannelDriver::new(adc, batt_pin, &config).expect("m");

        Ok(Self {
            adc_channel: adc_pin,
        })
    }

    pub fn read(&mut self) -> Result<Voltage, EspError> {
        let mut m = MeanWithError::new();
        for _ in 0..8 {
            m.add(self.adc_channel.read()? as f64 * 2_f64);
        }
        Ok(Voltage::new::<millivolt>(m.mean() as f32))
    }

    pub fn soc(voltage: Voltage) -> f32 {
        static SOC_TABLE: [f32; 21] = [
            0_f32, 5_f32, 10_f32, 15_f32, 20_f32, 25_f32, 30_f32, 35_f32, 40_f32, 45_f32, 50_f32,
            55_f32, 60_f32, 65_f32, 70_f32, 75_f32, 80_f32, 85_f32, 90_f32, 95_f32, 100_f32,
        ];
        static V_TABLE: [f32; 21] = [
            3.27_f32, 3.61_f32, 3.69_f32, 3.71_f32, 3.73_f32, 3.75_f32, 3.77_f32, 3.79_f32,
            3.8_f32, 3.82_f32, 3.84_f32, 3.85_f32, 3.87_f32, 3.91_f32, 3.95_f32, 3.98_f32,
            4.02_f32, 4.08_f32, 4.11_f32, 4.15_f32, 4.35_f32,
        ];

        let v = voltage.get::<volt>();
        let interpoloated_value = interp::interp(&V_TABLE, &SOC_TABLE, v, &InterpMode::FirstLast);

        // log::info!("Voltage: {:?}, interpolated soc {}", v, interpoloated_value);
        interpoloated_value / 100_f32
    }
}

pub fn display_create(peripherals: &mut DisplayPerepherials) -> Result<M5Display, EspError> {
    log::info!("Initializing display");
    let spi = SpiDeviceDriver::new_single(
        &mut peripherals.spi,
        &mut peripherals.sck,
        &mut peripherals.mo,
        Some(&mut peripherals.mi),
        Some(&mut peripherals.cs),
        &SpiDriverConfig::new(),
        &esp_idf_svc::hal::spi::config::Config::new().baudrate(MegaHertz(10).into()),
    )?;

    let mut display_interface = IT8951SPIInterface::new(
        spi,
        PinDriver::input(&mut peripherals.busy).unwrap(),
        DummyPin::new_high(),
        Delay::new_default(),
    );

    display_interface
        .wait_while_busy()
        .expect("Timeout display init");

    let epd: M5Display = IT8951::new_with_mcs(
        display_interface,
        Config::default(),
        MemoryConverterSetting {
            rotation: it8951::memory_converter_settings::MemoryConverterRotation::Rotate90,
            ..Default::default()
        },
    )
    .init(2300)
    .expect("Unable to initialize display");

    log::info!("Initialized display: {:?}", epd.get_dev_info());

    Ok(epd)
}
