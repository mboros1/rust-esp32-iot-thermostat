use std::time::Duration;

use esp_idf_hal::delay::FreeRtos;

use esp_idf_hal::gpio::PinDriver;
use esp_idf_hal::io::Write;
use esp_idf_hal::onewire::{OWAddress, OWCommand, OWDriver};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::http::server::EspHttpServer;
use esp_idf_svc::wifi::{AuthMethod, BlockingWifi, ClientConfiguration, EspWifi};
use esp_idf_sys::{EspError, ESP_ERR_INVALID_ARG};

// TODO: refactor so thermostat logic is in tasks
// TODO: save temperature to a deque like structure to calculate rolling averages
// TODO: think about thermostat temp controls; like compare the delta/rolling average of temp
// calculations, how much to go over/under the set temperature

fn main() {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();
    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("Starting Electric Dreams Super AI Crypto Thermostat");

    let peripherals = Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take().unwrap();

    let ssid = ""; // TODO: fill in with my actual SSID
    let pwd = ""; // TODO: fill in with my actual pwd
    let _wifi = match wifi(ssid, pwd, peripherals.modem, sysloop) {
        Ok(wifi) => wifi,
        Err(e) => {
            log::error!("Failed to connect to wifi: {:?}", e);
            return;
        }
    };

    let mut server = match EspHttpServer::new(&esp_idf_svc::http::server::Configuration::default())
    {
        Ok(srv) => srv,
        Err(e) => {
            log::error!("Failed to start http server: {:?}", e);
            return;
        }
    };

    server
        .fn_handler("/index.html", esp_idf_svc::http::Method::Get, |request| {
            request
                .into_ok_response()
                .unwrap()
                .write_all(b"<html><body>Hello world!</body></html>")
        })
        .unwrap();

    let pin = peripherals.pins.gpio5;
    let pin4 = peripherals.pins.gpio4;

    let mut relay_input = PinDriver::output(pin4).unwrap();

    let channel = peripherals.rmt.channel0;

    let mut onewire_bus: OWDriver = OWDriver::new(pin, channel).unwrap();

    let device = match onewire_bus.search().unwrap().next() {
        Some(Ok(dev)) => {
            log::info!(
                "Found device: {:?}, family code = {}",
                dev,
                dev.family_code()
            );
            dev
        }
        Some(Err(err)) => {
            log::error!("Error occured searching for device: {:?}", err);
            return;
        }
        None => {
            log::info!("No device found");
            return;
        }
    };

    let mut relay_high = false;

    loop {
        ds18b20_trigger_temp_conversion(&device, &onewire_bus).unwrap();
        let temp = ds18b20_get_temperature(&device, &onewire_bus).unwrap();
        log::info!("Temperature: {} C, {} F", temp, c_to_f(temp));

        if relay_high {
            relay_input.set_high().unwrap();
            log::info!("Relay input high!");
            relay_high = false;
        } else {
            relay_input.set_low().unwrap();
            log::info!("Relay input low!");
            relay_high = true;
        }

        FreeRtos::delay_ms(3000);
    }
}

pub fn c_to_f(celsius: f32) -> f32 {
    celsius * 9.0 / 5.0 + 32.0
}

fn ds18b20_send_command(addr: &OWAddress, bus: &OWDriver, cmd: u8) -> Result<(), EspError> {
    let mut buf = [0; 10];
    buf[0] = OWCommand::MatchRom as _;
    let addr = addr.address().to_le_bytes();
    buf[1..9].copy_from_slice(&addr);
    buf[9] = cmd;

    bus.write(&buf)
}

#[allow(dead_code)]
#[repr(u8)]
enum Ds18b20Command {
    ConvertTemp = 0x44,
    WriteScratch = 0x4E,
    ReadScratch = 0xBE,
}

fn ds18b20_trigger_temp_conversion(addr: &OWAddress, bus: &OWDriver) -> Result<(), EspError> {
    // reset bus and check if the ds18b20 is present
    bus.reset()?;

    ds18b20_send_command(addr, bus, Ds18b20Command::ConvertTemp as u8)?;

    // delay proper time for temp conversion,
    // assume max resolution (12-bits)
    std::thread::sleep(Duration::from_millis(800));

    Ok(())
}

fn ds18b20_get_temperature(addr: &OWAddress, bus: &OWDriver) -> Result<f32, EspError> {
    bus.reset()?;

    ds18b20_send_command(addr, bus, Ds18b20Command::ReadScratch as u8)?;

    let mut buf = [0u8; 9];
    bus.read(&mut buf)?;
    let lsb = buf[0];
    let msb = buf[1];

    let temp_raw: u16 = (u16::from(msb) << 8) | u16::from(lsb);

    Ok(f32::from(temp_raw) / 16.0)
}

pub fn wifi(
    ssid: &str,
    pass: &str,
    modem: impl esp_idf_svc::hal::peripheral::Peripheral<P = esp_idf_svc::hal::modem::Modem> + 'static,
    sysloop: EspSystemEventLoop,
) -> Result<Box<EspWifi<'static>>, EspError> {
    let mut auth_method = AuthMethod::WPA2Personal;
    if ssid.is_empty() {
        log::error!("Missing WiFi name");
        return Err(EspError::from(ESP_ERR_INVALID_ARG).unwrap());
    }
    if pass.is_empty() {
        auth_method = AuthMethod::None;
        log::info!("Wifi password is empty");
    }
    let nvs = esp_idf_svc::nvs::EspDefaultNvsPartition::take().ok();
    let mut esp_wifi = EspWifi::new(modem, sysloop.clone(), nvs)?;

    let mut wifi = BlockingWifi::wrap(&mut esp_wifi, sysloop)?;

    wifi.set_configuration(&esp_idf_svc::wifi::Configuration::Client(
        ClientConfiguration::default(),
    ))?;

    log::info!("Starting wifi...");

    wifi.start()?;

    log::info!("Scanning...");

    let ap_infos = wifi.scan()?;

    let ours = ap_infos.into_iter().find(|a| a.ssid == ssid);

    let channel = if let Some(ours) = ours {
        log::info!(
            "Found configured access point {} on channel {}",
            ssid,
            ours.channel
        );
        Some(ours.channel)
    } else {
        log::info!(
            "Configured access point {} not found during scanning, will go with unknown channel",
            ssid
        );
        None
    };

    wifi.set_configuration(&esp_idf_svc::wifi::Configuration::Client(
        ClientConfiguration {
            ssid: ssid
                .try_into()
                .expect("Could not parse the given SSID into WiFi config"),
            password: pass
                .try_into()
                .expect("Could not parse the given password into WiFi config"),
            channel,
            auth_method,
            ..Default::default()
        },
    ))?;

    log::info!("Connecting wifi...");

    wifi.connect()?;

    log::info!("Waiting for DHCP lease...");

    wifi.wait_netif_up()?;

    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;

    log::info!("Wifi DHCP info: {:?}", ip_info);

    Ok(Box::new(esp_wifi))
}
