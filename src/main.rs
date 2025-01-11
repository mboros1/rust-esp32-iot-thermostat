use esp_idf_svc::hal::{gpio::{InputOutput, PinDriver, Gpio5}, delay::FreeRtos, prelude::Peripherals};
use embedded_hal::digital::v2::{InputPin, OutputPin, StatefulOutputPin};
use onewire::{ds18b20, DeviceSearch, OneWire};

struct OpenDrainPin<'a> {
    driver: PinDriver<'a, Gpio5, InputOutput>,
}

impl<'a> OpenDrainPin<'a> {
    pub fn new(driver: PinDriver<'a, Gpio5, InputOutput>) -> Self {
        Self {driver }
    }
}

impl<'a> OutputPin for OpenDrainPin<'a> {
    type Error = esp_idf_svc::hal::gpio::GpioError;

    fn set_low(&mut self) -> Result<(), Self::Error> {
        self.driver.set_low().map_err(|e| esp_idf_svc::hal::gpio::GpioError::other(e))
    }

    fn set_high(&mut self) -> Result<(), Self::Error> {
        self.driver.set_high().map_err(|e| esp_idf_svc::hal::gpio::GpioError::other(e))
    }

}
impl<'a> InputPin for OpenDrainPin<'a> {
    type Error = esp_idf_svc::hal::gpio::GpioError;

    fn is_high(&self) -> Result<bool, Self::Error> {
        Ok(self.driver.is_high())
    }

    fn is_low(&self) -> Result<bool, Self::Error> {
        Ok(self.driver.is_low())
    }
}

impl<'a> StatefulOutputPin for OpenDrainPin<'a> {
    fn is_set_high(&self) -> Result<bool, Self::Error> {
        Ok(self.driver.is_set_high())
    }

    fn is_set_low(&self) -> Result<bool, Self::Error> {
        Ok(self.driver.is_set_low())
    }
}


fn main() {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();
    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("Starting Electric Dreams Super AI Crypto Thermostat");

    let peripherals = Peripherals::take().unwrap();

    let pin = peripherals.pins.gpio5;

    let driver = PinDriver::input_output_od(pin).unwrap();
    let mut open_drain_pin = OpenDrainPin::new(driver);

    let mut wire = OneWire::new(&mut open_drain_pin, false);

    let mut search = DeviceSearch::new();
    let mut delay = FreeRtos;

    log::info!("scanning devices...");
    // TODO: no devices are getting connected; try debug the physical hardware with the multimeter
    //       
    let mut i = 1;
    while let Some(device) = wire.search_next(&mut search, &mut delay).unwrap() {
        log::info!("checking device {}...", i);
        i += 1;
        match device.address[0] {
            ds18b20::FAMILY_CODE => {
                log::info!("thermostat found..");
                let thermostat = ds18b20::DS18B20::new::<esp_idf_svc::hal::sys::EspError>(device).unwrap();
                let resolution = thermostat.measure_temperature(&mut wire, &mut delay).unwrap();
                FreeRtos::delay_ms(resolution.time_ms().into());
                let temperature = thermostat.read_temperature(&mut wire, &mut delay).unwrap();

                log::info!("Temperature: {} C", temperature);

            },
            _ => {
                log::info!("unknown found..");
                // unknown device type
            }
        }
    }

}

