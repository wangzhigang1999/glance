pub mod mibeacon;
pub mod protocol;

enum Advertisement {
    Scale(protocol::Reading, i32, std::time::Instant),
    Clock(Vec<u8>),
}

use std::{
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

use esp_idf_svc::{
    bt::{
        ble::gap::{BleGapEvent, EspBleGap, GapSearchEvent, ScanParams, ScanType},
        Ble, BtDriver, BtStatus,
    },
    hal::modem::BluetoothModem,
    nvs::{EspDefaultNvsPartition, EspNvs},
};
use serde::{Deserialize, Serialize};

pub const ADDRESS: &str = "70:87:9E:41:24:A0";
const MAC: [u8; 6] = [0x70, 0x87, 0x9e, 0x41, 0x24, 0xa0];
const HISTORY_LIMIT: usize = 16;
pub type SharedScale = Arc<Mutex<Snapshot>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Measurement {
    #[serde(default)]
    pub event_id: String,
    pub kg: f32,
    pub unix_secs: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Snapshot {
    pub clock: mibeacon::Snapshot,
    pub scanning: bool,
    pub live_kg: Option<f32>,
    pub stable: bool,
    pub last_seen_unix: Option<i64>,
    pub rssi: Option<i32>,
    pub packets: u64,
    pub history: Vec<Measurement>,
    pub error: String,
    #[serde(skip)]
    pub last_seen: Option<Instant>,
}

pub fn spawn(modem: BluetoothModem<'static>, partition: EspDefaultNvsPartition) -> SharedScale {
    let shared = Arc::new(Mutex::new(Snapshot::default()));
    let output = shared.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("scale".into())
        .stack_size(8192)
        .spawn(move || {
            if let Err(e) = run(modem, partition, output.clone()) {
                log::error!("Scale receiver: {e:#}");
                let mut s = output.lock().unwrap();
                s.scanning = false;
                s.error = format!("{e:#}");
            }
        })
    {
        shared.lock().unwrap().error = format!("Scale thread: {e}");
    }
    shared
}

fn run(
    modem: BluetoothModem<'static>,
    partition: EspDefaultNvsPartition,
    shared: SharedScale,
) -> anyhow::Result<()> {
    let nvs = EspNvs::new(partition.clone(), "scale", true)?;
    let mut buf = [0u8; 2048];
    if let Some(data) = nvs.get_blob("history", &mut buf)? {
        match serde_json::from_slice::<Vec<Measurement>>(data) {
            Ok(mut history) => {
                let original_len = history.len();
                history.retain(|r| protocol::valid_weight(r.kg));
                if history.len() > HISTORY_LIMIT {
                    history.drain(..history.len() - HISTORY_LIMIT);
                }
                if history.len() != original_len {
                    nvs.set_blob("history", &serde_json::to_vec(&history)?)?;
                }
                shared.lock().unwrap().history = history;
            }
            Err(e) => log::warn!("Ignoring invalid scale history: {e}"),
        }
    }
    let gap = EspBleGap::new(BtDriver::<Ble>::new(modem, Some(partition))?)?;
    let mut clock = mibeacon::Decoder::new();
    shared.lock().unwrap().clock.configured = clock.configured();
    let (sender, receiver) = mpsc::sync_channel(32);
    let (control_tx, control_rx) = mpsc::channel();
    gap.subscribe(move |event| match event {
        BleGapEvent::ScanParameterConfigured(status) => {
            let _ = control_tx.send((false, status));
        }
        BleGapEvent::ScanStarted(status) => {
            let _ = control_tx.send((true, status));
        }
        BleGapEvent::ScanResult(GapSearchEvent::InquiryResult(result))
            if result.bda.addr() == MAC =>
        {
            if let Some(reading) = result.ble_adv.and_then(protocol::advertisement) {
                let _ = sender.try_send(Advertisement::Scale(reading, result.rssi, Instant::now()));
            }
        }
        BleGapEvent::ScanResult(GapSearchEvent::InquiryResult(result))
            if result.bda.addr() == mibeacon::MAC =>
        {
            if let Some(adv) = result.ble_adv {
                let _ = sender.try_send(Advertisement::Clock(adv.to_vec()));
            }
        }
        _ => {}
    })?;
    gap.set_scan_params(&ScanParams {
        scan_type: ScanType::Passive,
        scan_interval: 160, // 100ms
        scan_window: 128,   // 80ms; coexistence scheduler shares airtime with Wi-Fi
        ..Default::default()
    })?;
    let (_, status) = control_rx.recv_timeout(Duration::from_secs(10))?;
    anyhow::ensure!(
        status == BtStatus::Success,
        "BLE scan parameters: {status:?}"
    );
    gap.start_scanning(0)?;
    let (started, status) = control_rx.recv_timeout(Duration::from_secs(10))?;
    anyhow::ensure!(
        started && status == BtStatus::Success,
        "BLE scan start: {status:?}"
    );
    shared.lock().unwrap().scanning = true;
    log::info!("Scale listening: {ADDRESS}, passive BLE + Wi-Fi coexistence");
    let boot = Instant::now();
    let mut session = protocol::Session::default();
    let mut replay_guard = true;
    loop {
        let (reading, rssi, received) = match receiver.recv()? {
            Advertisement::Scale(reading, rssi, received) => (reading, rssi, received),
            Advertisement::Clock(adv) => {
                clock.receive(&adv, &mut shared.lock().unwrap().clock);
                continue;
            }
        };
        let now = crate::net::time::unix_secs();
        if !reading.stable || reading.removed {
            replay_guard = false;
        }
        let save = session.accept(reading, boot.elapsed().as_millis() as u64);
        let history = {
            let mut s = shared.lock().unwrap();
            s.live_kg = if reading.removed || !protocol::valid_weight(reading.kg) {
                None
            } else {
                Some(reading.kg)
            };
            s.stable = reading.stable && !reading.removed && protocol::valid_weight(reading.kg);
            s.rssi = Some(rssi);
            s.last_seen = Some(received);
            s.last_seen_unix = now;
            s.packets += 1;
            // Reboot replay guard: same value within 90s of a persisted measurement.
            let duplicate = replay_guard
                && s.history.last().is_some_and(|last| {
                    (last.kg - reading.kg).abs() < 0.001
                        && now
                            .zip(last.unix_secs)
                            .is_some_and(|(n, t)| n >= t && n - t < 90)
                });
            if save && !duplicate {
                if s.history.len() == HISTORY_LIMIT {
                    s.history.remove(0);
                }
                s.history.push(Measurement {
                    event_id: format!(
                        "{:08x}{:08x}",
                        unsafe { esp_idf_svc::sys::esp_random() },
                        unsafe { esp_idf_svc::sys::esp_random() }
                    ),
                    kg: reading.kg,
                    unix_secs: now,
                });
                Some(s.history.clone())
            } else {
                None
            }
        };
        if save {
            replay_guard = false;
        }
        if let Some(history) = history {
            let bytes = serde_json::to_vec(&history)?;
            match nvs.set_blob("history", &bytes) {
                Ok(_) => {
                    shared.lock().unwrap().error.clear();
                    log::info!("Scale saved stable reading, history={}", history.len());
                }
                Err(e) => {
                    shared.lock().unwrap().error = format!("Save failed: {e}");
                    log::error!("Scale NVS save failed: {e}");
                }
            }
        }
    }
}

pub fn register(
    server: &mut esp_idf_svc::http::server::EspHttpServer<'static>,
    shared: SharedScale,
) -> anyhow::Result<()> {
    use esp_idf_svc::{http::Method, io::Write};
    server.fn_handler("/api/weight", Method::Get, move |req| -> anyhow::Result<()> {
        let body = {
            let s = shared.lock().unwrap();
            serde_json::json!({"device_address": ADDRESS, "age_seconds": s.last_seen.map(|t| t.elapsed().as_secs()), "scale": &*s}).to_string()
        };
        req.into_response(200, Some("OK"), &[("Content-Type","application/json"),("Cache-Control","no-store")])?.write_all(body.as_bytes())?;
        Ok(())
    })?;
    Ok(())
}
