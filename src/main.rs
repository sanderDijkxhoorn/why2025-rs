use std::thread;
use std::time::Duration;

fn main() {
    // Required so runtime patches from esp-idf-sys are linked in.
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    loop {
        log::info!("WHY2025-RS Base Firmware Online");
        thread::sleep(Duration::from_secs(2));
    }
}
