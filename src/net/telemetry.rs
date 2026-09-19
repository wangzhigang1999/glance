//! Private WSS telemetry; durable weight outbox, bounded RAM climate backlog.
use std::{
    collections::VecDeque,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

use esp_idf_svc::{
    handle::RawHandle,
    mqtt::client::{
        EspMqttClient, EventPayload, LwtConfiguration, MqttClientConfiguration,
        MqttProtocolVersion, QoS,
    },
    nvs::{EspDefaultNvsPartition, EspNvs},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::system_http::SharedSystem;
use crate::scale::SharedScale;

const CONFIG: &str = include_str!(concat!(env!("OUT_DIR"), "/iot_config.json"));
const ROOT: &str = "home/v1/devices/rlcd-01";
#[derive(Deserialize)]
struct Credentials {
    uri: String,
    username: String,
    password: String,
    device_id: String,
}
#[derive(Default, Serialize)]
pub struct Status {
    configured: bool,
    connected: bool,
    queued_climate: usize,
    pending_weight: bool,
    broker_acks: u64,
    climate_dropped: u64,
    reconnects: u64,
    error: String,
}
pub type SharedTelemetry = Arc<Mutex<Status>>;
#[derive(Clone, Serialize, Deserialize)]
struct Pending {
    key: String,
    payload: String,
}
#[derive(Default, Serialize, Deserialize)]
struct Durable {
    seen: Vec<String>,
    pending: Option<Pending>,
}
enum Event {
    Connected,
    Disconnected,
    Published(u32),
}
fn random_id() -> String {
    format!(
        "{:08x}{:08x}{:08x}{:08x}",
        unsafe { esp_idf_svc::sys::esp_random() },
        unsafe { esp_idf_svc::sys::esp_random() },
        unsafe { esp_idf_svc::sys::esp_random() },
        unsafe { esp_idf_svc::sys::esp_random() }
    )
}
fn persist(nvs: &EspNvs<esp_idf_svc::nvs::NvsDefault>, state: &Durable) -> anyhow::Result<()> {
    nvs.set_blob("outbox", &serde_json::to_vec(state)?)?;
    Ok(())
}
fn sample(
    boot_id: &str,
    seq: &mut u64,
    ts_ms: Option<i64>,
    uptime_ms: u64,
    values: Value,
) -> String {
    *seq += 1;
    json!({"schema":1,"boot_id":boot_id,"seq":*seq,"ts_ms":ts_ms,"uptime_ms":uptime_ms,"values":values}).to_string()
}
fn sensor(v: Option<f32>, low: f32, high: f32) -> Option<f32> {
    v.filter(|v| v.is_finite() && *v >= low && *v <= high)
}
pub fn spawn(
    partition: EspDefaultNvsPartition,
    system: SharedSystem,
    scale: SharedScale,
) -> SharedTelemetry {
    let status = Arc::new(Mutex::new(Status::default()));
    let output = status.clone();
    let worker_output = output.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("telemetry".into())
        .stack_size(8192)
        .spawn(move || {
            if let Err(e) = run(partition, system, scale, worker_output.clone()) {
                let mut s = worker_output.lock().unwrap();
                s.connected = false;
                s.error = format!("{e:#}");
                log::error!("Telemetry worker stopped: {e:#}");
            }
        })
    {
        output.lock().unwrap().error = format!("Spawn: {e}");
    }
    status
}
fn run(
    partition: EspDefaultNvsPartition,
    system: SharedSystem,
    scale: SharedScale,
    status: SharedTelemetry,
) -> anyhow::Result<()> {
    let Ok(credentials) = serde_json::from_str::<Credentials>(CONFIG) else {
        status.lock().unwrap().error = "No private MQTT configuration".into();
        return Ok(());
    };
    anyhow::ensure!(
        credentials.uri == "wss://iot.bupt.site/mqtt"
            && credentials.device_id == "rlcd-01"
            && credentials.username == "rlcd-01",
        "Unexpected MQTT configuration"
    );
    status.lock().unwrap().configured = true;
    let nvs = EspNvs::new(partition, "telemetry", true)?;
    let mut bytes = vec![0u8; 4096];
    let mut durable: Durable = match nvs.get_blob("outbox", &mut bytes)? {
        Some(data) => serde_json::from_slice(data)?,
        None => Durable::default(),
    };
    drop(bytes);
    let boot_id = random_id();
    let boot_unix = crate::net::time::unix_secs();
    let mut seq = 0;
    let mut climate: VecDeque<String> = VecDeque::new();
    let mut next_sample = Instant::now();
    let mut clock_sent = [None; 3];
    let mut client: Option<EspMqttClient<'static>> = None;
    let (tx, rx) = mpsc::channel();
    let mut connected = false;
    let mut connecting = false;
    let mut deadline = Instant::now();
    let mut retry_at = Instant::now();
    let mut backoff = 1u64;
    let mut inflight: Option<(u32, bool, Instant)> = None;
    loop {
        let now = Instant::now();
        let sys = system.read().unwrap().clone();
        // Only enqueue newly authenticated clock measurements. Preserve receipt time,
        // never relabel an old clock value as a new sample every minute.
        let clock = scale.lock().unwrap().clock.clone();
        for (index, (field, reading)) in [
            ("mijia_temperature_c", clock.temperature),
            ("mijia_humidity_pct", clock.humidity),
            ("mijia_battery_pct", clock.battery),
        ]
        .into_iter()
        .enumerate()
        {
            if let Some(reading) = reading {
                if clock_sent[index] != Some(reading.received)
                    && reading.received.elapsed().as_secs() <= 900
                    && climate.len() < 60
                {
                    let ts = reading.unix_secs.or_else(|| {
                        sys.unix_secs
                            .map(|now| now - reading.received.elapsed().as_secs() as i64)
                    });
                    if ts.is_some() {
                        climate.push_back(sample(
                            &boot_id,
                            &mut seq,
                            ts.map(|t| t * 1000),
                            sys.uptime_s * 1000,
                            json!({field:reading.value}),
                        ));
                        clock_sent[index] = Some(reading.received);
                    }
                }
            }
        }
        // A complete stable record remains in the scale's NVS history until copied here.
        if durable.pending.is_none() && scale.lock().unwrap().scanning {
            let history = scale.lock().unwrap().history.clone();
            for (index, reading) in history.iter().enumerate() {
                if !crate::scale::protocol::valid_weight(reading.kg) {
                    continue;
                }
                let key = if !reading.event_id.is_empty() {
                    reading.event_id.clone()
                } else {
                    format!(
                        "old-{}-{}",
                        reading.unix_secs.unwrap_or(index as i64),
                        reading.kg.to_bits()
                    )
                };
                if durable.seen.contains(&key) {
                    continue;
                }
                let payload = sample(
                    &boot_id,
                    &mut seq,
                    reading.unix_secs.map(|t| t * 1000),
                    if reading
                        .unix_secs
                        .zip(boot_unix)
                        .is_some_and(|(t, b)| t >= b)
                    {
                        unsafe { esp_idf_svc::sys::esp_timer_get_time() as u64 / 1000 }
                    } else {
                        0
                    },
                    json!({"weight_kg":reading.kg,"weight_stable":true,"weight_removed":false}),
                );
                durable.pending = Some(Pending { key, payload });
                persist(&nvs, &durable)?;
                break;
            }
        }
        if now >= next_sample && sys.sample_count > 0 {
            if climate.len() == 60 {
                // Preserve the front record if it is in flight; shed newest backlog instead.
                climate.pop_back();
                status.lock().unwrap().climate_dropped += 1;
            }
            climate.push_back(sample(&boot_id, &mut seq, sys.unix_secs.map(|t|t*1000), sys.uptime_s*1000,
                json!({"temperature_c":sensor(sys.temp_c,-80.0,150.0),"humidity_pct":sensor(sys.humid_pct,0.0,100.0),
                       "chip_temperature_c":sys.chip_temp_c,"battery_mv":sys.battery_mv,"battery_pct":sys.battery_pct,
                       "wifi_rssi":sys.wifi_rssi,"heap_free":sys.heap_free,"usb_plugged":sys.usb_plugged})));
            next_sample = now + Duration::from_secs(60);
        }
        while let Ok(event) = rx.try_recv() {
            match event {
                Event::Connected => {
                    connected = true;
                    connecting = false;
                    backoff = 1;
                    status.lock().unwrap().error.clear();
                    if let Some(c) = client.as_mut() {
                        c.enqueue(
                            &format!("{ROOT}/status"),
                            QoS::AtLeastOnce,
                            true,
                            b"{\"online\":true}",
                        )?;
                        c.enqueue(&format!("{ROOT}/info"), QoS::AtLeastOnce, true,
                            json!({"firmware":env!("CARGO_PKG_VERSION"),"boot_id":boot_id,"schema":1,"climate_interval_s":60,"weight_min_kg":1}).to_string().as_bytes())?;
                    }
                    log::info!("Telemetry WSS connected (TLS verified)");
                }
                Event::Disconnected => {
                    connected = false;
                    connecting = false;
                    let jitter =
                        unsafe { esp_idf_svc::sys::esp_random() } as u64 % (backoff * 250 + 1);
                    retry_at =
                        now + Duration::from_millis((backoff * 750 + jitter).clamp(1000, 60000));
                    backoff = (backoff * 2).min(60);
                    status.lock().unwrap().error = "Disconnected; retry pending".into();
                }
                Event::Published(id) => {
                    if let Some((expected, weight, _)) = inflight {
                        if id == expected {
                            if weight {
                                if let Some(p) = durable.pending.take() {
                                    durable.seen.push(p.key);
                                    if durable.seen.len() > 32 {
                                        durable.seen.remove(0);
                                    }
                                    persist(&nvs, &durable)?;
                                }
                            } else {
                                climate.pop_front();
                            }
                            inflight = None;
                            status.lock().unwrap().broker_acks += 1;
                        }
                    }
                }
            }
        }
        // Certificate checks require a valid clock; never fall back to insecure TLS.
        if !connected
            && !connecting
            && now >= retry_at
            && sys.wifi_connected
            && sys.unix_secs.is_some()
        {
            if let Some(c) = client.as_ref() {
                let result = unsafe { esp_idf_svc::sys::esp_mqtt_client_reconnect(c.handle()) };
                if result != 0 {
                    log::warn!("MQTT reconnect error {result}");
                }
            } else {
                let sender = tx.clone();
                let topic = format!("{ROOT}/status");
                let conf = MqttClientConfiguration {
                    protocol_version: Some(MqttProtocolVersion::V3_1_1),
                    client_id: Some("rlcd-01"),
                    username: Some(&credentials.username),
                    password: Some(&credentials.password),
                    keep_alive_interval: Some(Duration::from_secs(60)),
                    reconnect_timeout: None,
                    network_timeout: Duration::from_secs(10),
                    disable_clean_session: true,
                    task_stack: 6144,
                    buffer_size: 1024,
                    out_buffer_size: 1024,
                    outbox_limit: Some(8192),
                    crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
                    skip_cert_common_name_check: false,
                    lwt: Some(LwtConfiguration {
                        topic: &topic,
                        payload: b"{\"online\":false}",
                        qos: QoS::AtLeastOnce,
                        retain: true,
                    }),
                    ..Default::default()
                };
                client = Some(EspMqttClient::new_cb(
                    &credentials.uri,
                    &conf,
                    move |event| {
                        let e = match event.payload() {
                            EventPayload::Connected(_) => Some(Event::Connected),
                            EventPayload::Disconnected => Some(Event::Disconnected),
                            EventPayload::Published(id) => Some(Event::Published(id)),
                            _ => None,
                        };
                        if let Some(e) = e {
                            let _ = sender.send(e);
                        }
                    },
                )?);
            }
            connecting = true;
            deadline = now + Duration::from_secs(20);
            status.lock().unwrap().reconnects += 1;
        }
        if connecting && now >= deadline {
            connecting = false;
            retry_at = now + Duration::from_secs(backoff);
            backoff = (backoff * 2).min(60);
        }
        // ESP-MQTT retains QoS1 messages across reconnects. Only rebuild after a long
        // unacknowledged interval, keeping the original serialized payload and ID.
        if inflight.is_some_and(|(_, _, sent)| sent.elapsed() > Duration::from_secs(180)) {
            client = None;
            connected = false;
            connecting = false;
            inflight = None;
            while rx.try_recv().is_ok() {}
            retry_at = now + Duration::from_secs(1);
        }
        if connected && inflight.is_none() {
            let pending = durable
                .pending
                .as_ref()
                .map(|p| (true, p.payload.as_str()))
                .or_else(|| climate.front().map(|p| (false, p.as_str())));
            if let (Some((weight, payload)), Some(c)) = (pending, client.as_mut()) {
                match c.enqueue(
                    &format!("{ROOT}/telemetry"),
                    QoS::AtLeastOnce,
                    false,
                    payload.as_bytes(),
                ) {
                    Ok(id) => inflight = Some((id, weight, Instant::now())),
                    Err(e) => status.lock().unwrap().error = format!("Enqueue: {e}"),
                }
            }
        }
        {
            let mut s = status.lock().unwrap();
            s.connected = connected;
            s.queued_climate = climate.len();
            s.pending_weight = durable.pending.is_some();
        }

        std::thread::sleep(Duration::from_millis(200));
    }
}
pub fn register(
    server: &mut esp_idf_svc::http::server::EspHttpServer<'static>,
    shared: SharedTelemetry,
) -> anyhow::Result<()> {
    use esp_idf_svc::{http::Method, io::Write};
    server.fn_handler(
        "/api/telemetry",
        Method::Get,
        move |req| -> anyhow::Result<()> {
            let body = serde_json::to_vec(&*shared.lock().unwrap())?;
            req.into_response(
                200,
                Some("OK"),
                &[
                    ("Content-Type", "application/json"),
                    ("Cache-Control", "no-store"),
                ],
            )?
            .write_all(&body)?;
            Ok(())
        },
    )?;
    Ok(())
}
