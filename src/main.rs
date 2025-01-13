use std::time::Duration;

use esp_idf_hal::delay::FreeRtos;


#[cfg(all(
    esp_idf_soc_rmt_supported,
    not(feature = "rmt-legacy"),
    esp_idf_comp_espressif__onewire_bus_enabled,
))]
use esp_idf_hal::onewire::{OWAddress, OWCommand, OWDriver};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_sys::EspError;

fn main() {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();
    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("Starting Electric Dreams Super AI Crypto Thermostat");

    let peripherals = Peripherals::take().unwrap();
    let pin = peripherals.pins.gpio5;
    let channel = peripherals.rmt.channel0;

    let mut onewire_bus: OWDriver = OWDriver::new(pin, channel).unwrap();


    let device = {
        let mut search = onewire_bus.search().unwrap();
        search.next()
    };

    if device.is_none() {
        println!("no device found");
        return;
    }

    let device = device.unwrap();
    if let Err(err) = device {
        println!("error occured searching for device, err={}", err);
        return;
    }

    let device = device.unwrap();
    println!(
        "Found device: {:?}, family code = {}",
        device,
        device.family_code()
    );
}

