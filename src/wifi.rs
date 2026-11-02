use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::io::{self, BufRead};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use esp_idf_hal::modem::Modem;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::{EspDefaultNvs, EspDefaultNvsPartition};
use esp_idf_svc::wifi::{
    AccessPointInfo, AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi,
};
use why2025_ui::{Network, WifiCommand, WifiSnapshot, WifiStatus};

const PROFILE_KEY: &str = "profile";
const PROFILE_VERSION: u8 = 1;
const PROFILE_HEADER_LEN: usize = 4;
const MAX_PROFILE_LEN: usize = PROFILE_HEADER_LEN + 32 + 63;
const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(2);
const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(5);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);

#[derive(Clone)]
struct Profile {
    ssid: String,
    password: String,
    auth_method: AuthMethod,
}

struct WifiManager {
    wifi: BlockingWifi<EspWifi<'static>>,
    nvs: EspDefaultNvs,
    scan_results: Vec<AccessPointInfo>,
    snapshot: WifiSnapshot,
    updates: Sender<WifiSnapshot>,
    profile: Option<Profile>,
    auto_reconnect: bool,
    next_retry_at: Option<Instant>,
    retry_delay: Duration,
}

pub fn spawn_console(sender: Sender<WifiCommand>) -> io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("wifi-console".into())
        .stack_size(4096)
        .spawn(move || {
            let stdin = io::stdin();
            loop {
                let mut line = String::new();
                match stdin.lock().read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) if line.len() > 160 => {
                        log::warn!("Wi-Fi command is too long");
                        continue;
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(100));
                    }
                    Err(error) => {
                        log::error!("Wi-Fi serial input failed: {error}");
                        break;
                    }
                }

                match parse_command(&line) {
                    Ok(Some(command)) => {
                        if sender.send(command).is_err() {
                            log::warn!("Wi-Fi manager is not accepting serial commands");
                            break;
                        }
                    }
                    Ok(None) => {}
                    Err(()) => log::warn!(
                        "Wi-Fi commands: wifi scan | wifi connect <number> [password] | wifi disconnect | wifi forget"
                    ),
                }
            }
        })
}

fn parse_command(line: &str) -> Result<Option<WifiCommand>, ()> {
    let line = line.trim_end_matches(['\r', '\n']);
    let Some(command) = line.strip_prefix("wifi ") else {
        return Ok(None);
    };

    if command == "scan" {
        return Ok(Some(WifiCommand::Scan));
    }
    if command == "disconnect" {
        return Ok(Some(WifiCommand::Disconnect));
    }
    if command == "forget" {
        return Ok(Some(WifiCommand::Forget));
    }
    if command == "help" {
        return Err(());
    }

    let Some(connect) = command.strip_prefix("connect ") else {
        return Err(());
    };
    let (index, password) = match connect.split_once(' ') {
        Some((index, password)) => (index, password),
        None => (connect, ""),
    };
    let index = index.parse::<usize>().map_err(|_| ())?;
    if index == 0 || password.len() > 63 {
        return Err(());
    }

    Ok(Some(WifiCommand::Connect {
        index,
        password: password.to_owned(),
    }))
}

pub fn run(
    modem: Modem<'static>,
    commands: Receiver<WifiCommand>,
    updates: Sender<WifiSnapshot>,
) -> Result<(), String> {
    let startup_status = WifiSnapshot {
        detail: Some("Waiting for ESP32-C6 hosted radio".to_owned()),
        ..WifiSnapshot::default()
    };
    updates
        .send(startup_status)
        .map_err(|error| format!("Wi-Fi startup status: {error}"))?;
    log::info!("Initializing ESP-Hosted Wi-Fi radio over SDIO");

    let system_loop =
        EspSystemEventLoop::take().map_err(|error| format!("system event loop: {error}"))?;
    let partition =
        EspDefaultNvsPartition::take().map_err(|error| format!("NVS initialization: {error}"))?;
    let nvs = EspDefaultNvs::new(partition.clone(), "wifi", true)
        .map_err(|error| format!("Wi-Fi settings namespace: {error}"))?;
    log::info!("Creating ESP-Hosted Wi-Fi driver");
    let wifi = EspWifi::new(modem, system_loop.clone(), Some(partition))
        .map_err(|error| format!("Wi-Fi driver initialization: {error}"))?;
    log::info!("ESP-Hosted Wi-Fi driver created");
    let wifi = BlockingWifi::wrap(wifi, system_loop)
        .map_err(|error| format!("Wi-Fi event loop: {error}"))?;

    let mut manager = WifiManager {
        wifi,
        nvs,
        scan_results: Vec::new(),
        snapshot: WifiSnapshot {
            detail: Some("Waiting for ESP32-C6 hosted radio".to_owned()),
            ..WifiSnapshot::default()
        },
        updates,
        profile: None,
        auto_reconnect: false,
        next_retry_at: None,
        retry_delay: INITIAL_RETRY_DELAY,
    };

    manager.publish();
    manager
        .wifi
        .set_configuration(&Configuration::Client(ClientConfiguration::default()))
        .map_err(|error| format!("Wi-Fi station configuration: {error}"))?;
    log::info!("Starting Wi-Fi station");
    manager
        .wifi
        .start()
        .map_err(|error| format!("Wi-Fi driver start: {error}"))?;

    log::info!("Wi-Fi station started; beginning initial scan");
    let initial_scan_succeeded = match manager.scan() {
        Ok(()) => true,
        Err(error) => {
            log::error!("Initial Wi-Fi scan failed: {error}");
            manager.set_status(WifiStatus::Error, None, None, Some(error));
            false
        }
    };

    match load_profile(&manager.nvs) {
        Ok(Some(profile)) => {
            if let Err(error) = manager.connect_profile(profile, false) {
                log::warn!("Saved Wi-Fi network connection failed: {error}");
            }
        }
        Ok(None) if initial_scan_succeeded => {
            manager.set_status(WifiStatus::Ready, None, None, None)
        }
        Ok(None) => {}
        Err(error) => {
            log::error!("Saved Wi-Fi profile could not be read: {error}");
            manager.set_status(WifiStatus::Error, None, None, Some(error));
        }
    }

    let mut last_status_poll = Instant::now();
    let mut receiver_open = true;
    loop {
        if receiver_open {
            match commands.recv_timeout(Duration::from_millis(200)) {
                Ok(command) => {
                    if let Err(error) = manager.handle(command) {
                        log::error!("Wi-Fi command failed: {error}");
                        manager.set_status(
                            WifiStatus::Error,
                            manager.snapshot.ssid.clone(),
                            None,
                            Some(error),
                        );
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    receiver_open = false;
                    log::warn!("Wi-Fi serial command input closed");
                }
            }
        } else {
            thread::sleep(Duration::from_millis(200));
        }

        if last_status_poll.elapsed() >= STATUS_POLL_INTERVAL {
            last_status_poll = Instant::now();
            if let Err(error) = manager.refresh_link() {
                log::error!("Wi-Fi status check failed: {error}");
                manager.set_status(
                    WifiStatus::Error,
                    manager.snapshot.ssid.clone(),
                    None,
                    Some(error),
                );
            }
        }

        if manager
            .next_retry_at
            .is_some_and(|retry_at| Instant::now() >= retry_at)
        {
            if let Some(profile) = manager.profile.clone() {
                if let Err(error) = manager.connect_profile(profile, false) {
                    log::warn!("Wi-Fi reconnect failed: {error}");
                }
            } else {
                manager.next_retry_at = None;
            }
        }
    }
}

impl WifiManager {
    fn handle(&mut self, command: WifiCommand) -> Result<(), String> {
        match command {
            WifiCommand::Scan => self.scan(),
            WifiCommand::Connect { index, password } => self.connect_scan_result(index, password),
            WifiCommand::Disconnect => {
                self.auto_reconnect = false;
                self.next_retry_at = None;
                if self
                    .wifi
                    .is_connected()
                    .map_err(|error| error.to_string())?
                {
                    self.wifi.disconnect().map_err(|error| error.to_string())?;
                }
                self.set_status(
                    WifiStatus::Disconnected,
                    self.snapshot.ssid.clone(),
                    None,
                    None,
                );
                Ok(())
            }
            WifiCommand::Forget => {
                if self
                    .wifi
                    .is_connected()
                    .map_err(|error| error.to_string())?
                {
                    self.wifi.disconnect().map_err(|error| error.to_string())?;
                }
                self.nvs
                    .remove(PROFILE_KEY)
                    .map_err(|error| format!("removing saved Wi-Fi profile: {error}"))?;
                self.profile = None;
                self.auto_reconnect = false;
                self.next_retry_at = None;
                self.set_status(WifiStatus::Ready, None, None, None);
                Ok(())
            }
        }
    }

    fn scan(&mut self) -> Result<(), String> {
        self.set_status(
            WifiStatus::Scanning,
            self.snapshot.ssid.clone(),
            self.snapshot.ip.clone(),
            None,
        );
        let results = self
            .wifi
            .wifi_mut()
            .scan()
            .map_err(|error| format!("Wi-Fi scan: {error}"))?;
        let mut strongest_by_ssid = BTreeMap::new();
        let mut hidden_networks = Vec::new();
        for access_point in results {
            let ssid = access_point.ssid.to_string();
            if ssid.is_empty() {
                hidden_networks.push(access_point);
                continue;
            }
            match strongest_by_ssid.entry(ssid) {
                Entry::Vacant(entry) => {
                    entry.insert(access_point);
                }
                Entry::Occupied(mut entry) => {
                    if access_point.signal_strength > entry.get().signal_strength {
                        entry.insert(access_point);
                    }
                }
            }
        }
        let mut results: Vec<AccessPointInfo> = strongest_by_ssid.into_values().collect();
        results.append(&mut hidden_networks);
        results.sort_by(|left, right| right.signal_strength.cmp(&left.signal_strength));
        self.scan_results = results;
        for (index, access_point) in self.scan_results.iter().enumerate() {
            log::info!(
                "Wi-Fi AP {:02}: {} {} dBm ({})",
                index + 1,
                display_ssid(access_point.ssid.as_str()),
                access_point.signal_strength,
                auth_label(access_point.auth_method.as_ref())
            );
        }
        self.snapshot.networks = self
            .scan_results
            .iter()
            .map(|access_point| {
                let auth_method = access_point.auth_method.clone();
                Network {
                    ssid: access_point.ssid.to_string(),
                    signal_strength: access_point.signal_strength,
                    security: auth_label(auth_method.as_ref()),
                    connectable: supported_auth(auth_method.as_ref()),
                }
            })
            .collect();
        self.publish();
        self.refresh_link()
    }

    fn connect_scan_result(&mut self, index: usize, password: String) -> Result<(), String> {
        let access_point = self
            .scan_results
            .get(index - 1)
            .ok_or_else(|| format!("network {index} is not in the latest scan"))?;
        let auth_method = access_point
            .auth_method
            .clone()
            .ok_or_else(|| "network security could not be identified".to_owned())?;
        if !supported_auth(Some(&auth_method)) {
            return Err(format!(
                "{} authentication is not supported",
                auth_label(Some(&auth_method))
            ));
        }

        self.connect_profile(
            Profile {
                ssid: access_point.ssid.to_string(),
                password,
                auth_method,
            },
            true,
        )
    }

    fn connect_profile(&mut self, profile: Profile, save: bool) -> Result<(), String> {
        validate_profile(&profile)?;
        let ssid = profile.ssid.clone();
        if save {
            save_profile(&self.nvs, &profile)?;
        }
        self.profile = Some(profile.clone());
        self.auto_reconnect = true;
        self.next_retry_at = None;
        self.set_status(WifiStatus::Connecting, Some(ssid.clone()), None, None);

        let client = ClientConfiguration {
            ssid: profile
                .ssid
                .as_str()
                .try_into()
                .map_err(|_| "SSID exceeds the Wi-Fi limit".to_owned())?,
            password: profile
                .password
                .as_str()
                .try_into()
                .map_err(|_| "password exceeds the Wi-Fi limit".to_owned())?,
            auth_method: profile.auth_method,
            ..Default::default()
        };

        let result = self
            .wifi
            .set_configuration(&Configuration::Client(client))
            .and_then(|()| self.wifi.connect())
            .and_then(|()| self.wifi.wait_netif_up())
            .and_then(|()| {
                self.wifi
                    .wifi()
                    .sta_netif()
                    .get_ip_info()
                    .map(|ip_info| ip_info.ip.to_string())
            });
        match result {
            Ok(ip) => {
                self.retry_delay = INITIAL_RETRY_DELAY;
                self.set_status(WifiStatus::Connected, Some(ssid), Some(ip), None);
                log::info!("Connected to Wi-Fi network");
                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                self.set_status(WifiStatus::Error, Some(ssid), None, Some(message.clone()));
                self.schedule_reconnect();
                Err(message)
            }
        }
    }

    fn refresh_link(&mut self) -> Result<(), String> {
        if self.wifi.is_up().map_err(|error| error.to_string())? {
            let ip = self
                .wifi
                .wifi()
                .sta_netif()
                .get_ip_info()
                .map_err(|error| error.to_string())?
                .ip
                .to_string();
            if self.snapshot.status != WifiStatus::Connected
                || self.snapshot.ip.as_deref() != Some(&ip)
            {
                let profile = load_profile(&self.nvs)?;
                let ssid = self
                    .snapshot
                    .ssid
                    .clone()
                    .or_else(|| profile.map(|profile| profile.ssid));
                self.set_status(WifiStatus::Connected, ssid, Some(ip), None);
            }
        } else if self.snapshot.status == WifiStatus::Scanning
            || self.snapshot.status == WifiStatus::Starting
        {
            self.set_status(WifiStatus::Ready, self.snapshot.ssid.clone(), None, None);
        } else if self.snapshot.status == WifiStatus::Connected {
            self.set_status(
                WifiStatus::Disconnected,
                self.snapshot.ssid.clone(),
                None,
                Some("Connection lost".into()),
            );
            self.schedule_reconnect();
        }
        Ok(())
    }

    fn schedule_reconnect(&mut self) {
        if self.auto_reconnect && self.profile.is_some() {
            self.next_retry_at = Some(Instant::now() + self.retry_delay);
            self.retry_delay = self.retry_delay.saturating_mul(2).min(MAX_RETRY_DELAY);
        }
    }

    fn set_status(
        &mut self,
        status: WifiStatus,
        ssid: Option<String>,
        ip: Option<String>,
        detail: Option<String>,
    ) {
        self.snapshot.status = status;
        self.snapshot.ssid = ssid;
        self.snapshot.ip = ip;
        self.snapshot.detail = detail;
        self.publish();
    }

    fn publish(&self) {
        if let Err(error) = self.updates.send(self.snapshot.clone()) {
            log::warn!("Wi-Fi status display is unavailable: {error}");
        }
    }
}

fn validate_profile(profile: &Profile) -> Result<(), String> {
    if profile.ssid.is_empty() || profile.ssid.len() > 32 {
        return Err("SSID must contain 1 to 32 bytes".into());
    }
    if profile.password.len() > 63 {
        return Err("password must be at most 63 bytes".into());
    }
    match profile.auth_method {
        AuthMethod::None if !profile.password.is_empty() => {
            Err("an open network must not have a password".into())
        }
        AuthMethod::None => Ok(()),
        _ if !(8..=63).contains(&profile.password.len()) => {
            Err("password must contain 8 to 63 bytes".into())
        }
        _ => Ok(()),
    }
}

fn supported_auth(auth_method: Option<&AuthMethod>) -> bool {
    matches!(
        auth_method,
        Some(
            AuthMethod::None
                | AuthMethod::WPA
                | AuthMethod::WPA2Personal
                | AuthMethod::WPAWPA2Personal
                | AuthMethod::WPA3Personal
                | AuthMethod::WPA2WPA3Personal
        )
    )
}

fn auth_label(auth_method: Option<&AuthMethod>) -> &'static str {
    match auth_method {
        Some(AuthMethod::None) => "OPEN",
        Some(AuthMethod::WEP) => "WEP",
        Some(AuthMethod::WPA) => "WPA",
        Some(AuthMethod::WPA2Personal) => "WPA2",
        Some(AuthMethod::WPAWPA2Personal) => "WPA/WPA2",
        Some(AuthMethod::WPA2Enterprise) => "ENTERPRISE",
        Some(AuthMethod::WPA3Personal) => "WPA3",
        Some(AuthMethod::WPA2WPA3Personal) => "WPA2/3",
        Some(AuthMethod::WAPIPersonal) => "WAPI",
        None => "UNKNOWN",
    }
}

fn display_ssid(ssid: &str) -> String {
    ssid.chars()
        .map(|character| {
            if character.is_ascii_graphic() || character == ' ' {
                character
            } else {
                '?'
            }
        })
        .collect()
}

fn auth_code(auth_method: &AuthMethod) -> Option<u8> {
    match auth_method {
        AuthMethod::None => Some(0),
        AuthMethod::WPA => Some(1),
        AuthMethod::WPA2Personal => Some(2),
        AuthMethod::WPAWPA2Personal => Some(3),
        AuthMethod::WPA3Personal => Some(4),
        AuthMethod::WPA2WPA3Personal => Some(5),
        _ => None,
    }
}

fn decode_auth(code: u8) -> Option<AuthMethod> {
    match code {
        0 => Some(AuthMethod::None),
        1 => Some(AuthMethod::WPA),
        2 => Some(AuthMethod::WPA2Personal),
        3 => Some(AuthMethod::WPAWPA2Personal),
        4 => Some(AuthMethod::WPA3Personal),
        5 => Some(AuthMethod::WPA2WPA3Personal),
        _ => None,
    }
}

fn save_profile(nvs: &EspDefaultNvs, profile: &Profile) -> Result<(), String> {
    let auth = auth_code(&profile.auth_method)
        .ok_or_else(|| "unsupported Wi-Fi authentication type".to_owned())?;
    let mut encoded =
        Vec::with_capacity(PROFILE_HEADER_LEN + profile.ssid.len() + profile.password.len());
    encoded.extend_from_slice(&[
        PROFILE_VERSION,
        profile.ssid.len() as u8,
        profile.password.len() as u8,
        auth,
    ]);
    encoded.extend_from_slice(profile.ssid.as_bytes());
    encoded.extend_from_slice(profile.password.as_bytes());
    nvs.set_blob(PROFILE_KEY, &encoded)
        .map_err(|error| format!("saving Wi-Fi profile: {error}"))
}

fn load_profile(nvs: &EspDefaultNvs) -> Result<Option<Profile>, String> {
    let Some(length) = nvs
        .blob_len(PROFILE_KEY)
        .map_err(|error| format!("reading Wi-Fi profile length: {error}"))?
    else {
        return Ok(None);
    };
    if !(PROFILE_HEADER_LEN..=MAX_PROFILE_LEN).contains(&length) {
        return Err("saved Wi-Fi profile has an invalid length".into());
    }

    let mut buffer = [0; MAX_PROFILE_LEN];
    let encoded = nvs
        .get_blob(PROFILE_KEY, &mut buffer)
        .map_err(|error| format!("reading Wi-Fi profile: {error}"))?
        .ok_or_else(|| "saved Wi-Fi profile disappeared".to_owned())?;
    if encoded.len() < PROFILE_HEADER_LEN || encoded[0] != PROFILE_VERSION {
        return Err("saved Wi-Fi profile has an unsupported format".into());
    }

    let ssid_len = encoded[1] as usize;
    let password_len = encoded[2] as usize;
    if ssid_len == 0
        || ssid_len > 32
        || password_len > 63
        || encoded.len() != PROFILE_HEADER_LEN + ssid_len + password_len
    {
        return Err("saved Wi-Fi profile has invalid field lengths".into());
    }
    let ssid = std::str::from_utf8(&encoded[PROFILE_HEADER_LEN..PROFILE_HEADER_LEN + ssid_len])
        .map_err(|_| "saved Wi-Fi SSID is not valid UTF-8".to_owned())?
        .to_owned();
    let password_start = PROFILE_HEADER_LEN + ssid_len;
    let password = std::str::from_utf8(&encoded[password_start..])
        .map_err(|_| "saved Wi-Fi password is not valid UTF-8".to_owned())?
        .to_owned();
    let auth_method = decode_auth(encoded[3])
        .ok_or_else(|| "saved Wi-Fi authentication is invalid".to_owned())?;

    let profile = Profile {
        ssid,
        password,
        auth_method,
    };
    validate_profile(&profile)?;
    Ok(Some(profile))
}
