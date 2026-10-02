//! One bounded, read-only LAN worker. Device tokens never enter read APIs, logs or telemetry.
use std::{
    collections::BTreeMap,
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use super::miio_config::{self as config, Device};
use anyhow::{ensure, Result};
use esp_idf_svc::{
    http::{server::EspHttpServer, Method},
    io::{Read, Write},
    nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault},
    sys,
};
use serde::Serialize;
use serde_json::{json, Value};

use super::{miio_wire as wire, system_http::SharedSystem};
const CONFIG: &str = include_str!(concat!(env!("OUT_DIR"), "/mijia_config.json"));
const HELLO: [u8; 32] = {
    let mut h = [255; 32];
    h[0] = 0x21;
    h[1] = 0x31;
    h[2] = 0;
    h[3] = 32;
    h
};
struct Store {
    nvs: EspNvs<NvsDefault>,
    devices: Vec<Device>,
    revision: u64,
}
static STORE: OnceLock<Mutex<Store>> = OnceLock::new();

pub fn initialize(partition: EspDefaultNvsPartition) -> Result<()> {
    let nvs = EspNvs::new(partition, "mijia_config", true)?;
    let mut bytes = vec![0u8; config::MAX_BYTES];
    let devices: Vec<Device> = match nvs.get_blob("devices_v1", &mut bytes)? {
        Some(raw) => serde_json::from_slice(raw)?,
        None => serde_json::from_str(CONFIG)?,
    };
    config::validate(&devices)?;
    // Seed configuration remains a recovery fallback until the first successful import.
    let _ = STORE.set(Mutex::new(Store {
        nvs,
        devices,
        revision: 1,
    }));
    Ok(())
}

fn state_for(devices: &[Device]) -> Vec<DeviceState> {
    devices
        .iter()
        .map(|d| DeviceState {
            id: d.id.clone(),
            name: d.name.clone(),
            model: d.model.clone(),
            ip: d.ip.to_string(),
            ..Default::default()
        })
        .collect()
}
#[derive(Clone, Default, Serialize, Debug)]
pub struct DeviceState {
    pub id: String,
    pub name: String,
    pub model: String,
    pub ip: String,
    pub online: bool,
    pub values: BTreeMap<String, Value>,
    pub generation: u64,
    pub sampled_at: Option<i64>,
    pub uptime_ms: u64,
    pub error: String,
    pub firmware: Option<String>,
}
#[derive(Default, Serialize, Clone, Debug)]
pub struct Snapshot {
    pub devices: Vec<DeviceState>,
    pub error: String,
    pub poll_interval_s: u64,
}
pub type SharedHome = Arc<Mutex<Snapshot>>;
fn md5(input: &[u8]) -> Result<[u8; 16]> {
    let mut out = [0; 16];
    ensure!(
        unsafe { sys::mbedtls_md5(input.as_ptr(), input.len(), out.as_mut_ptr()) } == 0,
        "MD5 failed"
    );
    Ok(out)
}
fn aes(data: &[u8], key: &[u8; 16], iv: &[u8; 16], encrypt: bool) -> Result<Vec<u8>> {
    let mut output = vec![0; data.len()];
    let mut iv = *iv;
    unsafe {
        let mut ctx = std::mem::zeroed::<sys::esp_aes_context>();
        sys::esp_aes_init(&mut ctx);
        let mut code = sys::esp_aes_setkey(&mut ctx, key.as_ptr(), 128);
        if code == 0 {
            code = sys::esp_aes_crypt_cbc(
                &mut ctx,
                if encrypt { 1 } else { 0 },
                data.len(),
                iv.as_mut_ptr(),
                data.as_ptr(),
                output.as_mut_ptr(),
            );
        }
        sys::esp_aes_free(&mut ctx);
        ensure!(code == 0, "AES failed");
    }
    Ok(output)
}
fn token(text: &str) -> Result<[u8; 16]> {
    ensure!(text.len() == 32 && text.is_ascii(), "Invalid token");
    let mut out = [0; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)?;
    }
    Ok(out)
}
fn request(d: &Device, method: &str, params: Value) -> Result<Value> {
    ensure!(
        matches!(method, "miIO.info" | "get_properties" | "get_prop"),
        "Read-only method required"
    );
    let secret = token(&d.token)?;
    let key = md5(&secret)?;
    let iv = md5(&[key, secret].concat())?;
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
    socket.connect((d.ip, 54321))?;
    socket.send(&HELLO)?;
    let mut buffer = vec![0u8; wire::MAX_PACKET + 1];
    let n = socket.recv(&mut buffer)?;
    ensure!(
        n == 32 && wire::valid_packet(&buffer[..n], d.did, false),
        "Unexpected device handshake"
    );
    let id = (unsafe { sys::esp_random() } & 0x7fffffff).max(1);
    let mut plaintext = serde_json::to_vec(&json!({"id":id,"method":method,"params":params}))?;
    let padding = 16 - plaintext.len() % 16;
    plaintext.resize(plaintext.len() + padding, padding as u8);
    let body = aes(&plaintext, &key, &iv, true)?;
    ensure!(body.len() + 32 <= wire::MAX_PACKET, "Request too large");
    let mut packet = buffer[..32].to_vec();
    packet[2..4].copy_from_slice(&((body.len() + 32) as u16).to_be_bytes());
    packet[16..32].copy_from_slice(&secret);
    packet.extend_from_slice(&body);
    let digest = md5(&packet)?;
    packet[16..32].copy_from_slice(&digest);
    socket.send(&packet)?;
    let n = socket.recv(&mut buffer)?;
    ensure!(
        wire::valid_packet(&buffer[..n], d.did, true),
        "Invalid device response"
    );
    let signature: [u8; 16] = buffer[16..32].try_into().unwrap();
    buffer[16..32].copy_from_slice(&secret);
    let expected = md5(&buffer[..n])?;
    ensure!(
        signature
            .iter()
            .zip(expected)
            .fold(0u8, |v, (a, b)| v | (*a ^ b))
            == 0,
        "Device authentication failed"
    );
    let decrypted = aes(&buffer[32..n], &key, &iv, false)?;
    let clear = wire::unpad(&decrypted).ok_or_else(|| anyhow::anyhow!("Invalid padding"))?;
    let clear = clear.strip_suffix(&[0]).unwrap_or(clear);
    let response: Value = serde_json::from_slice(clear)?;
    ensure!(
        response["id"].as_u64() == Some(id as u64) && response.get("error").is_none(),
        "Read request rejected"
    );
    Ok(response["result"].clone())
}
fn read(d: &Device) -> Result<(BTreeMap<String, Value>, Option<String>)> {
    let mut values = BTreeMap::new();
    if d.properties.is_empty() {
        let info = request(d, "miIO.info", json!([]))?;
        return Ok((values, info["fw_ver"].as_str().map(str::to_string)));
    }
    let legacy = d.properties[0].legacy.len() > 0;
    let params = if legacy {
        Value::Array(d.properties.iter().map(|p| json!(p.legacy)).collect())
    } else {
        Value::Array(
            d.properties
                .iter()
                .map(|p| json!({"did":d.did.to_string(),"siid":p.siid,"piid":p.piid}))
                .collect(),
        )
    };
    let result = request(
        d,
        if legacy { "get_prop" } else { "get_properties" },
        params,
    )?;
    let result = result
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid property response"))?;
    let mut valid = 0;
    for (index, p) in d.properties.iter().enumerate() {
        let mut value = if legacy {
            result.get(index).cloned().unwrap_or(Value::Null)
        } else {
            result
                .iter()
                .find(|r| {
                    r["siid"].as_u64() == Some(p.siid as u64)
                        && r["piid"].as_u64() == Some(p.piid as u64)
                        && r["code"].as_i64() == Some(0)
                })
                .map(|r| r["value"].clone())
                .unwrap_or(Value::Null)
        };
        if p.key == "on" && legacy {
            value = match value.as_str() {
                Some("on") => json!(true),
                Some("off") => json!(false),
                _ => Value::Null,
            };
        }
        if !(value.is_number() || value.is_boolean()) {
            value = Value::Null;
        }
        if !value.is_null() {
            valid += 1;
        }
        values.insert(p.key.clone(), value);
    }
    ensure!(valid > 0, "No readable properties");
    Ok((values, None))
}
fn discover(devices: &mut [Device]) -> Result<()> {
    let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    s.set_broadcast(true)?;
    s.set_read_timeout(Some(Duration::from_millis(250)))?;
    s.send_to(&HELLO, (Ipv4Addr::BROADCAST, 54321))?;
    let start = Instant::now();
    let mut packet = [0u8; 64];
    while start.elapsed() < Duration::from_secs(2) {
        if let Ok((32, SocketAddr::V4(addr))) = s.recv_from(&mut packet) {
            if addr.port() != 54321 || !addr.ip().is_private() {
                continue;
            }
            for d in devices.iter_mut() {
                if wire::valid_packet(&packet[..32], d.did, false) {
                    d.ip = *addr.ip();
                }
            }
        }
    }
    Ok(())
}
pub fn spawn(system: SharedSystem) -> SharedHome {
    let output = Arc::new(Mutex::new(Snapshot {
        poll_interval_s: 60,
        ..Default::default()
    }));
    let Some(store) = STORE.get() else {
        output.lock().unwrap().error = "设备配置不可用".into();
        return output;
    };
    let (mut devices, mut revision) = {
        let saved = store.lock().unwrap();
        (saved.devices.clone(), saved.revision)
    };
    output.lock().unwrap().devices = state_for(&devices);
    let shared = output.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("mijia".into())
        .stack_size(8192)
        .spawn(move || {
            let mut next_discovery = Instant::now();
            loop {
                {
                    let saved = store.lock().unwrap();
                    if saved.revision != revision {
                        devices = saved.devices.clone();
                        revision = saved.revision;
                        shared.lock().unwrap().devices = state_for(&devices);
                        next_discovery = Instant::now();
                    }
                }
                if !system.read().unwrap().wifi_connected {
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
                let start = Instant::now();
                if start >= next_discovery {
                    let _ = discover(&mut devices);
                    next_discovery = start + Duration::from_secs(300);
                }
                for (i, d) in devices.iter().enumerate() {
                    // Retry one lost UDP response without holding the shared state lock.
                    let result = read(d).or_else(|_| {
                        std::thread::sleep(Duration::from_millis(200));
                        read(d)
                    });
                    let mut s = shared.lock().unwrap();
                    let state = &mut s.devices[i];
                    state.ip = d.ip.to_string();
                    state.generation = (unsafe { sys::esp_timer_get_time() } as u64).max(1);
                    state.sampled_at = super::time::unix_secs();
                    state.uptime_ms = unsafe { sys::esp_timer_get_time() as u64 / 1000 };
                    match result {
                        Ok((values, firmware)) => {
                            state.values = values;
                            state.firmware = firmware;
                            state.online = true;
                            state.error.clear();
                        }
                        Err(error) => {
                            log::warn!("MiHome {} read failed: {error}", d.id);
                            state.online = false;
                            state.values.clear();
                            state.error = "读取失败，将自动重试".into();
                        }
                    }
                }
                // Wake promptly for a configuration import, without restarting Wi-Fi or MQTT.
                while start.elapsed() < Duration::from_secs(60) {
                    if store.lock().unwrap().revision != revision {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        })
    {
        output.lock().unwrap().error = format!("Worker start failed: {e}");
    }
    output
}
pub fn snapshot(shared: &SharedHome) -> Snapshot {
    let mut s = shared.lock().unwrap().clone();
    let now = unsafe { sys::esp_timer_get_time() as u64 / 1000 };
    for d in &mut s.devices {
        if now.saturating_sub(d.uptime_ms) > 150_000 {
            d.online = false;
            d.values.clear();
            d.error = "数据已过期".into();
        }
    }
    s
}
pub fn register(server: &mut EspHttpServer<'static>, shared: SharedHome) -> Result<()> {
    server.fn_handler(
        "/api/home-devices/config",
        Method::Post,
        move |mut req| -> Result<()> {
            macro_rules! reply {
                ($code:expr, $message:expr) => {{
                    req.into_response(
                        $code,
                        None,
                        &[
                            ("Content-Type", "application/json"),
                            ("Cache-Control", "no-store"),
                        ],
                    )?
                    .write_all($message.as_bytes())?;
                    return Ok(());
                }};
            }
            if !super::admin_policy::allowed_origin(
                req.header("Origin"),
                req.header("Sec-Fetch-Site"),
                req.header("Host").unwrap_or(""),
            ) {
                reply!(403, r#"{"error":"不允许跨站配置"}"#);
            }
            if !super::admin_auth::authorized(req.header("X-Admin-Key").unwrap_or("")) {
                reply!(401, r#"{"error":"管理口令不正确"}"#);
            }
            let size = req
                .header("Content-Length")
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(0);
        if req.header("Transfer-Encoding").is_some()
            || size == 0
                || size > config::MAX_BYTES
                || req.header("Content-Type") != Some("application/json")
            {
                reply!(400, r#"{"error":"配置格式或大小不正确"}"#);
            }
            let mut raw = vec![0u8; size];
            req.read_exact(&mut raw)?;
            let incoming: Vec<Device> = match serde_json::from_slice(&raw) {
                Ok(v) => v,
                Err(_) => {
                    reply!(400, r#"{"error":"配置格式不正确"}"#);
                }
            };
            let Some(store) = STORE.get() else {
                reply!(503, r#"{"error":"存储尚未就绪"}"#);
            };
            let mut store = store.lock().unwrap();
            let next = match config::merged(&store.devices, incoming) {
                Ok(v) => v,
                Err(_) => {
                    reply!(400, r#"{"error":"设备配置无效或超过容量"}"#);
                }
            };
            let bytes = serde_json::to_vec(&next)?;
            if store.nvs.set_blob("devices_v1", &bytes).is_err() {
                reply!(507, r#"{"error":"存储空间不足；原设备配置保留"}"#);
            }
            store.devices = next;
            store.revision = store.revision.wrapping_add(1);
            reply!(200, r#"{"ok":true,"message":"已保存，即将自动采集"}"#);
        },
    )?;
    server.fn_handler("/api/home-devices", Method::Get, move |req| -> Result<()> {
        let body = serde_json::to_vec(&snapshot(&shared))?;
        req.into_response(
            200,
            Some("OK"),
            &[
                ("Content-Type", "application/json; charset=utf-8"),
                ("Cache-Control", "no-store"),
            ],
        )?
        .write_all(&body)?;
        Ok(())
    })?;
    Ok(())
}
