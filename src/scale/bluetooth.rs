//! One GATT central shares the existing Bluedroid radio with sensor scanning.
use std::{
    collections::VecDeque,
    sync::{mpsc, Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use anyhow::{anyhow, Result};
use esp_idf_svc::{
    bt::{
        ble::{
            gap::{BleAddrType, BleEncryption, EspBleGap},
            gatt::{
                client::{
                    CharacteristicElement, DescriptorElement, EspGattc, GattAuthReq, GattWriteType,
                    GattcEvent,
                },
                GattStatus, Property,
            },
        },
        BdAddr, Ble, BtDriver, BtUuid,
    },
    nvs::{EspDefaultNvsPartition, EspNvs},
};
use serde::{Deserialize, Serialize};

const ADDRESSES: [[u8; 6]; 2] = [
    [0x53, 0x14, 1, 2, 0x10, 0x79],
    [2, 1, 0x23, 0x27, 0xa5, 0x4f],
];
type Driver = Arc<BtDriver<'static, Ble>>;
type Client = EspGattc<'static, Ble, Driver>;
pub type Gap = EspBleGap<'static, Ble, Driver>;

#[derive(Clone, Default, Serialize, Deserialize)]
struct Config {
    enabled: [bool; 2],
    bindings: Vec<Binding>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Binding {
    key: String,
    action: String,
}
#[derive(Clone, Serialize)]
struct Seen {
    address: String,
    name: String,
    rssi: i32,
    packets: u32,
    age_s: u64,
    supported: bool,
    #[serde(skip)]
    seen: Instant,
}
#[derive(Serialize)]
struct Peer {
    name: &'static str,
    address: String,
    enabled: bool,
    state: String,
    error: String,
}
#[derive(Serialize)]
struct Report {
    seq: u64,
    key: String,
    hex: String,
    uptime_ms: u64,
}
#[derive(Serialize)]
struct State {
    devices: Vec<Seen>,
    peers: Vec<Peer>,
    reports: VecDeque<Report>,
    bindings: Vec<Binding>,
    learning: Option<String>,
    lamp_level: Option<u8>,
    last_action: String,
    last_gesture: String,
    error: String,
    dropped_events: u32,
    lamp_ack_ms: Option<u64>,
    gesture_ms: Option<u64>,
    lamp_timeouts: u32,
    last_lamp_report: String,
    lamp_battery_raw: Option<u8>,
    #[serde(skip)]
    battery_received: Option<Instant>,
    lamp_battery_age_s: Option<u64>,
}
static STATE: OnceLock<Mutex<State>> = OnceLock::new();
static COMMANDS: OnceLock<mpsc::SyncSender<Command>> = OnceLock::new();
static AUTH: OnceLock<mpsc::SyncSender<Event>> = OnceLock::new();
fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| {
        Mutex::new(State {
            devices: vec![],
            peers: ADDRESSES
                .iter()
                .enumerate()
                .map(|(i, a)| Peer {
                    name: if i == 0 {
                        "绿联翻页器"
                    } else {
                        "迈极炫灯"
                    },
                    address: address(a),
                    enabled: false,
                    state: "未连接".into(),
                    error: String::new(),
                })
                .collect(),
            reports: VecDeque::new(),
            bindings: vec![],
            learning: None,
            lamp_level: None,
            last_action: String::new(),
            last_gesture: String::new(),
            error: String::new(),
            dropped_events: 0,
            lamp_ack_ms: None,
            gesture_ms: None,
            lamp_timeouts: 0,
            last_lamp_report: String::new(),
            lamp_battery_raw: None,
            battery_received: None,
            lamp_battery_age_s: None,
        })
    })
}
fn address(a: &[u8; 6]) -> String {
    a.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[derive(Debug, Clone, Default)]
pub struct DisplaySnapshot {
    pub lamp_ready: bool,
    pub remote_ready: bool,
    pub level: Option<u8>,
    pub battery: Option<u8>,
    pub battery_age_s: Option<u64>,
}
pub fn display_snapshot() -> DisplaySnapshot {
    let s = state().lock().unwrap();
    let age = s.battery_received.map(|t| t.elapsed().as_secs());
    DisplaySnapshot {
        lamp_ready: s.peers[1].state == "已就绪",
        remote_ready: s.peers[0].state == "已就绪",
        level: s.lamp_level,
        battery: s.lamp_battery_raw.filter(|_| age.is_some_and(|a| a < 600)),
        battery_age_s: age,
    }
}
fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn observe(a: [u8; 6], rssi: i32, adv: &[u8]) {
    let mut name = String::new();
    let mut p = 0;
    while p < adv.len() {
        let n = adv[p] as usize;
        if n == 0 || p + n >= adv.len() {
            break;
        }
        if matches!(adv[p + 1], 8 | 9) {
            name = String::from_utf8_lossy(&adv[p + 2..p + n + 1])
                .chars()
                .take(32)
                .collect();
        }
        p += n + 1;
    }
    let mac = address(&a);
    let mut s = state().lock().unwrap();
    if let Some(d) = s.devices.iter_mut().find(|d| d.address == mac) {
        d.rssi = rssi;
        d.seen = Instant::now();
        d.packets = d.packets.saturating_add(1);
        if !name.is_empty() {
            d.name = name;
        }
        return;
    }
    if s.devices.len() >= 24 {
        let index = s
            .devices
            .iter()
            .enumerate()
            .max_by_key(|(_, d)| d.seen.elapsed())
            .map(|(i, _)| i)
            .unwrap();
        s.devices.remove(index);
    }
    s.devices.push(Seen {
        address: mac,
        name,
        rssi,
        packets: 1,
        age_s: 0,
        supported: ADDRESSES.contains(&a),
        seen: Instant::now(),
    });
}
pub fn auth(a: [u8; 6], ok: bool) {
    if let Some(tx) = AUTH.get() {
        let _ = tx.try_send(Event::Auth(a, ok));
    }
}
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Enable { device: usize, enabled: bool },
    Learn { action: String },
    Bind { key: String, action: String },
    CancelLearn,
    ClearBindings,
    Lamp { action: String },
}
enum Event {
    Registered(u8, bool),
    Open(u16, [u8; 6], GattStatus),
    Closed(u16),
    Search(u16, bool),
    Notify(u16, u16, Vec<u8>),
    RegisteredNotify(bool),
    Descriptor(u16, bool),
    Auth([u8; 6], bool),
    Write(u16, bool),
}
struct Link {
    conn: Option<u16>,
    ready: bool,
    retry: Instant,
    opening: bool,
    write: u16,
    subscriptions: usize,
    registered: Vec<u16>,
}
pub struct Manager {
    client: Client,
    iface: Option<u8>,
    rx: mpsc::Receiver<Event>,
    commands: mpsc::Receiver<Command>,
    links: [Link; 2],
    config: Config,
    nvs: EspNvs<esp_idf_svc::nvs::NvsDefault>,
    subscriptions: VecDeque<(usize, u16, u16)>,
    subscribing: Option<(usize, u16, u16, Instant)>,
    learning: Option<(String, Instant)>,
    gesture: super::remote_gesture::Gesture,
    last_report: Option<Instant>,
    first_report: Option<Instant>,
    level: Option<u8>,
    remembered: u8,
    pending: Option<(u8, Instant)>,
    battery_pending: Option<Instant>,
    battery_due: Instant,
    boot: Instant,
}
impl Manager {
    pub fn new(driver: Driver, partition: EspDefaultNvsPartition) -> Result<Self> {
        let nvs = EspNvs::new(partition, "bluetooth", true)?;
        let mut buf = [0u8; 2048];
        let config = nvs
            .get_blob("config", &mut buf)?
            .and_then(|b| serde_json::from_slice::<Config>(b).ok())
            .filter(|c| c.bindings.len() <= 3)
            .unwrap_or_default();
        let (tx, rx) = mpsc::sync_channel(48);
        let _ = AUTH.set(tx.clone());
        let (commands_tx, commands) = mpsc::sync_channel(8);
        let _ = COMMANDS.set(commands_tx);
        let client = EspGattc::new(driver)?;
        client.subscribe(move |(iface, e)| {
            let e = match e {
                GattcEvent::ClientRegistered { status, .. } => {
                    Event::Registered(iface, status == GattStatus::Ok)
                }
                GattcEvent::Open {
                    status,
                    conn_id,
                    addr,
                    ..
                } => Event::Open(conn_id, addr.addr(), status),
                GattcEvent::Disconnected { conn_id, .. } => Event::Closed(conn_id),
                GattcEvent::SearchComplete {
                    conn_id, status, ..
                } => Event::Search(conn_id, status == GattStatus::Ok),
                GattcEvent::Notify {
                    conn_id,
                    handle,
                    value,
                    ..
                } => Event::Notify(conn_id, handle, value.iter().take(64).copied().collect()),
                GattcEvent::RegisterNotify { status, .. } => {
                    Event::RegisteredNotify(status == GattStatus::Ok)
                }
                GattcEvent::WriteDescriptor {
                    conn_id, status, ..
                } => Event::Descriptor(conn_id, status == GattStatus::Ok),
                GattcEvent::WriteCharacteristic {
                    conn_id, status, ..
                } => Event::Write(conn_id, status == GattStatus::Ok),
                _ => return,
            };
            if tx.try_send(e).is_err() {
                let mut s = state().lock().unwrap();
                s.dropped_events = s.dropped_events.saturating_add(1);
            }
        })?;
        client.register_app(1)?;
        {
            let mut s = state().lock().unwrap();
            s.bindings = config.bindings.clone();
            for i in 0..2 {
                s.peers[i].enabled = config.enabled[i];
            }
        }
        Ok(Self {
            client,
            iface: None,
            rx,
            commands,
            links: std::array::from_fn(|_| Link {
                conn: None,
                ready: false,
                retry: Instant::now(),
                opening: false,
                write: 0,
                subscriptions: 0,
                registered: Vec::new(),
            }),
            config,
            nvs,
            subscriptions: VecDeque::new(),
            subscribing: None,
            learning: None,
            gesture: Default::default(),
            last_report: None,
            first_report: None,
            level: None,
            remembered: 10,
            pending: None,
            battery_pending: None,
            battery_due: Instant::now(),
            boot: Instant::now(),
        })
    }
    fn status(&self, i: usize, message: &str) {
        state().lock().unwrap().peers[i].state = message.into();
    }
    fn fail(&mut self, i: usize, message: &str) {
        if let Some(iface) = self.iface {
            for handle in self.links[i].registered.drain(..) {
                let _ =
                    self.client
                        .unregister_for_notify(iface, &BdAddr::from(ADDRESSES[i]), handle);
            }
        }
        {
            let mut s = state().lock().unwrap();
            s.peers[i].error = message.into();
            s.peers[i].state = "等待重连".into();
        }
        if let (Some(iface), Some(conn)) = (self.iface, self.links[i].conn.take()) {
            let _ = self.client.close(iface, conn);
        }
        self.links[i].ready = false;
        self.links[i].opening = false;
        self.links[i].retry = Instant::now() + Duration::from_secs(if i == 1 { 3 } else { 15 });
        self.subscriptions.retain(|(index, _, _)| *index != i);
        if self
            .subscribing
            .as_ref()
            .is_some_and(|(index, _, _, _)| *index == i)
        {
            self.subscribing = None;
        }
        if i == 1 {
            self.battery_pending = None;
            self.level = None;
            self.pending = None;
            let mut s = state().lock().unwrap();
            s.lamp_level = None;
            s.last_action = message.into();
        }
    }
    fn save(&mut self, next: Config) -> Result<()> {
        self.nvs.set_blob("config", &serde_json::to_vec(&next)?)?;
        self.config = next;
        state().lock().unwrap().bindings = self.config.bindings.clone();
        Ok(())
    }
    fn command(&mut self, c: Command) -> Result<()> {
        match c {
            Command::Enable { device, enabled } => {
                let mut next = self.config.clone();
                next.enabled[device] = enabled;
                self.save(next)?;
                state().lock().unwrap().peers[device].enabled = enabled;
                if !enabled {
                    self.fail(device, "");
                    self.status(device, "已停用");
                } else {
                    self.links[device].retry = Instant::now();
                }
            }
            Command::Learn { action } => {
                self.learning = Some((action.clone(), Instant::now()));
                state().lock().unwrap().learning = Some(action);
                self.gesture = Default::default();
                self.last_report = None;
                self.first_report = None;
            }
            Command::Bind { key, action } => {
                let mut next = self.config.clone();
                next.bindings.retain(|b| b.action != action && b.key != key);
                next.bindings.push(Binding { key, action });
                self.save(next)?;
            }
            Command::CancelLearn => {
                self.learning = None;
                state().lock().unwrap().learning = None;
            }
            Command::ClearBindings => {
                let mut c = self.config.clone();
                c.bindings.clear();
                self.save(c)?;
            }
            Command::Lamp { action } => self.lamp(&action)?,
        }
        Ok(())
    }
    fn lamp(&mut self, action: &str) -> Result<()> {
        anyhow::ensure!(self.links[1].ready, "灯尚未就绪");
        anyhow::ensure!(self.pending.is_none(), "正在等待灯确认");
        let current = self.level.unwrap_or(0);
        let level = match action {
            "toggle" => {
                if current > 0 {
                    0
                } else {
                    self.remembered
                }
            }
            "on" => self.remembered,
            "off" => 0,
            "up" => current.saturating_add(10).min(100),
            "down" => current.saturating_sub(10),
            _ => return Err(anyhow!("未知操作")),
        };
        let packet = super::lamp_protocol::packet(level).ok_or_else(|| anyhow!("亮度越界"))?;
        self.client.write_characteristic(
            self.iface.unwrap(),
            self.links[1].conn.unwrap(),
            self.links[1].write,
            &packet,
            GattWriteType::RequireResponse,
            GattAuthReq::None,
        )?;
        self.pending = Some((level, Instant::now()));
        state().lock().unwrap().last_action = "等待灯确认".into();
        Ok(())
    }
    fn discover(&mut self, i: usize) -> Result<()> {
        self.client
            .search_service(self.iface.unwrap(), self.links[i].conn.unwrap(), None)?;
        self.status(i, "发现服务");
        Ok(())
    }
    fn characteristics(&mut self, i: usize) -> Result<()> {
        let iface = self.iface.unwrap();
        let conn = self.links[i].conn.unwrap();
        let mut chars = [CharacteristicElement::new(); 32];
        let count = self
            .client
            .get_all_characteristics(iface, conn, 1, u16::MAX, 0, &mut chars)
            .map_err(|e| anyhow!("特征枚举: {e:?}"))?;
        let uuid = BtUuid::uuid16(if i == 0 { 0x2a4d } else { 0xffe0 });
        let mut n = 0;
        for c in &chars[..count] {
            if c.uuid() != uuid {
                continue;
            }
            if i == 1 {
                self.links[i].write = c.handle();
            }
            if !c.properties().contains(Property::Notify) {
                continue;
            }
            let mut descriptors = [DescriptorElement::new(); 1];
            let count = self
                .client
                .get_descriptor_by_char_handle(
                    iface,
                    conn,
                    c.handle(),
                    &BtUuid::uuid16(0x2902),
                    &mut descriptors,
                )
                .map_err(|e| anyhow!("通知描述符: {e:?}"))?;
            if count == 1 {
                self.subscriptions
                    .push_back((i, c.handle(), descriptors[0].handle()));
                n += 1;
            }
        }
        anyhow::ensure!(n > 0, "没有找到受支持的通知特征");
        self.links[i].subscriptions = n;
        log::info!("BLE peer {i}: {n} notification channels");
        self.status(i, "订阅按键或状态");
        Ok(())
    }
    fn notify(&mut self, i: usize, handle: u16, value: Vec<u8>) -> Result<()> {
        if i == 1 {
            state().lock().unwrap().last_lamp_report = hex(&value);
            log::info!("BLE lamp notify: {}", hex(&value));
            if self.battery_pending.is_some() {
                if let Some(raw) = super::lamp_protocol::battery(&value) {
                    self.battery_pending = None;
                    let mut s = state().lock().unwrap();
                    s.lamp_battery_raw = Some(raw);
                    s.battery_received = Some(Instant::now());
                }
            }
            if value == [0xde, 7, 0xb6, 0, 1, 0xb0, 0xed] {
                if let Some((level, sent)) = self.pending.take() {
                    self.level = Some(level);
                    if level > 0 {
                        self.remembered = level;
                    }
                    let mut s = state().lock().unwrap();
                    s.lamp_level = Some(level);
                    s.last_action = format!("灯已确认：{level}");
                    s.lamp_ack_ms = Some(sent.elapsed().as_millis() as u64);
                    s.error.clear();
                }
            }
            return Ok(());
        }
        let raw = hex(&value);
        let key = format!("{handle}:{raw}");
        {
            let mut s = state().lock().unwrap();
            let seq = s.reports.back().map_or(1, |r| r.seq + 1);
            if s.reports.len() >= 20 {
                s.reports.pop_front();
            }
            s.reports.push_back(Report {
                seq,
                key: key.clone(),
                hex: raw,
                uptime_ms: self.boot.elapsed().as_millis() as u64,
            });
        }
        self.gesture.push(handle, &value);
        self.first_report.get_or_insert_with(Instant::now);
        self.last_report = Some(Instant::now());
        Ok(())
    }
    fn handle_gesture(&mut self, key: String) -> Result<()> {
        state().lock().unwrap().last_gesture = key.clone();
        if let Some(start) = self.first_report.take() {
            state().lock().unwrap().gesture_ms = Some(start.elapsed().as_millis() as u64);
        }
        if let Some((action, _)) = self.learning.take() {
            let mut next = self.config.clone();
            next.bindings.retain(|b| b.action != action && b.key != key);
            next.bindings.push(Binding { key, action });
            self.save(next)?;
            state().lock().unwrap().learning = None;
            return Ok(());
        }
        if let Some(action) = self
            .config
            .bindings
            .iter()
            .find(|b| b.key == key)
            .map(|b| b.action.clone())
        {
            self.lamp(&action)?;
        }
        Ok(())
    }
    fn event(&mut self, e: Event, gap: &Gap) -> Result<()> {
        match e {
            Event::Registered(iface, ok) => {
                anyhow::ensure!(ok, "蓝牙客户端注册失败");
                self.iface = Some(iface);
            }
            Event::Open(conn, a, status) => {
                let _ = gap.start_scanning(0);
                if let Some(i) = ADDRESSES.iter().position(|x| *x == a) {
                    self.links[i].opening = false;
                    if status != GattStatus::Ok {
                        self.fail(i, &format!("连接失败: {status:?}"));
                    } else if !self.config.enabled[i] {
                        self.client.close(self.iface.unwrap(), conn)?;
                    } else {
                        self.links[i].conn = Some(conn);
                        self.links[i].retry = Instant::now() + Duration::from_secs(20);
                        state().lock().unwrap().peers[i].error.clear();
                        if i == 0 {
                            self.status(i, "配对加密");
                            gap.set_encryption(BdAddr::from(a), BleEncryption::EncryptionNoMitm)?;
                        } else {
                            self.discover(i)?;
                        }
                    }
                }
            }
            Event::Auth(a, ok) => {
                if a == ADDRESSES[0] && self.links[0].conn.is_some() {
                    if ok {
                        self.discover(0)?;
                    } else {
                        self.fail(0, "翻页器配对失败，请进入配对模式");
                    }
                }
            }
            Event::Closed(conn) => {
                if let Some(i) = self.links.iter().position(|l| l.conn == Some(conn)) {
                    self.fail(i, "连接已断开");
                    self.gesture = Default::default();
                    self.last_report = None;
                    self.first_report = None;
                }
            }
            Event::Search(conn, ok) => {
                if let Some(i) = self.links.iter().position(|l| l.conn == Some(conn)) {
                    if ok {
                        if let Err(e) = self.characteristics(i) {
                            self.fail(i, &e.to_string());
                        }
                    } else {
                        self.fail(i, "服务发现失败");
                    }
                }
            }
            Event::RegisteredNotify(ok) => {
                if let Some((i, _, desc, _)) = self.subscribing {
                    if ok {
                        self.client.write_descriptor(
                            self.iface.unwrap(),
                            self.links[i].conn.unwrap(),
                            desc,
                            &[1, 0],
                            GattWriteType::RequireResponse,
                            if i == 0 {
                                GattAuthReq::NoMitm
                            } else {
                                GattAuthReq::None
                            },
                        )?;
                    } else {
                        self.fail(i, "注册通知失败");
                    }
                }
            }
            Event::Descriptor(conn, ok) => {
                if let Some((i, _, _, _)) = self.subscribing {
                    if self.links[i].conn == Some(conn) {
                        self.subscribing = None;
                        if ok {
                            self.links[i].subscriptions =
                                self.links[i].subscriptions.saturating_sub(1);
                            if self.links[i].subscriptions == 0 {
                                self.links[i].ready = true;
                                if i == 1 {
                                    self.battery_due = Instant::now() + Duration::from_secs(2);
                                }
                                self.status(i, "已就绪");
                            }
                        } else {
                            self.fail(i, "订阅通知失败");
                        }
                    }
                }
            }
            Event::Write(conn, ok) => {
                if !ok && self.links[1].conn == Some(conn) {
                    if self.pending.is_none() && self.battery_pending.take().is_some() {
                        self.battery_due = Instant::now() + Duration::from_secs(60);
                    } else {
                        self.fail(1, "灯控写入失败");
                    }
                }
            }
            Event::Notify(conn, h, v) => {
                if let Some(i) = self.links.iter().position(|l| l.conn == Some(conn)) {
                    self.notify(i, h, v)?;
                }
            }
        }
        Ok(())
    }
    pub fn tick(&mut self, gap: &Gap) {
        for _ in 0..24 {
            let Ok(e) = self.rx.try_recv() else { break };
            if let Err(e) = self.event(e, gap) {
                state().lock().unwrap().error = e.to_string();
            }
        }
        if self
            .last_report
            .is_some_and(|t| t.elapsed() >= Duration::from_millis(self.gesture.quiet_ms()))
        {
            self.last_report = None;
            if let Some(key) = self.gesture.finish() {
                if let Err(e) = self.handle_gesture(key) {
                    state().lock().unwrap().error = e.to_string();
                }
            }
            self.first_report = None;
        }
        while let Ok(c) = self.commands.try_recv() {
            if let Err(e) = self.command(c) {
                state().lock().unwrap().error = e.to_string();
            } else {
                state().lock().unwrap().error.clear();
            }
        }
        if self
            .battery_pending
            .is_some_and(|t| t.elapsed() > Duration::from_secs(5))
        {
            self.battery_pending = None;
            // A failed telemetry read must not disconnect a working lamp control link.
            self.battery_due = Instant::now() + Duration::from_secs(60);
        }
        if self.links[1].ready
            && self.pending.is_none()
            && self.battery_pending.is_none()
            && self.last_report.is_none()
            && Instant::now() >= self.battery_due
        {
            self.battery_due = Instant::now() + Duration::from_secs(300);
            if self
                .client
                .write_characteristic(
                    self.iface.unwrap(),
                    self.links[1].conn.unwrap(),
                    self.links[1].write,
                    &super::lamp_protocol::BATTERY_QUERY,
                    GattWriteType::RequireResponse,
                    GattAuthReq::None,
                )
                .is_ok()
            {
                self.battery_pending = Some(Instant::now());
            }
        }
        if self
            .learning
            .as_ref()
            .is_some_and(|(_, t)| t.elapsed() > Duration::from_secs(20))
        {
            self.learning = None;
            let mut s = state().lock().unwrap();
            s.learning = None;
            s.error = "按键学习超时，请重试".into();
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|(_, t)| t.elapsed() > Duration::from_secs(5))
        {
            let mut s = state().lock().unwrap();
            s.lamp_timeouts = s.lamp_timeouts.saturating_add(1);
            drop(s);
            self.fail(1, "灯确认超时，状态未知");
        }
        if let Some((i, _, _, t)) = self.subscribing {
            if t.elapsed() > Duration::from_secs(8) {
                self.fail(i, "订阅超时");
            }
        }
        let Some(iface) = self.iface else { return };
        for i in 0..2 {
            if !self.config.enabled[i] {
                continue;
            }
            if self.links[i].conn.is_some()
                && !self.links[i].ready
                && Instant::now() > self.links[i].retry
            {
                self.fail(i, "连接初始化超时");
            }
            if self.links[i].conn.is_none()
                && !self.links[i].opening
                && Instant::now() >= self.links[i].retry
                && !self
                    .links
                    .iter()
                    .any(|l| l.opening || (l.conn.is_some() && !l.ready))
            {
                // Pause scanning only during connection establishment.
                let _ = gap.stop_scanning();
                std::thread::sleep(Duration::from_millis(200));
                self.links[i].opening = true;
                self.links[i].retry = Instant::now() + Duration::from_secs(20);
                self.status(i, "正在连接");
                if let Err(e) = self.client.enh_open(
                    iface,
                    &esp_idf_svc::bt::ble::gatt::client::GattCreateConnParams::new(
                        BdAddr::from(ADDRESSES[i]),
                        BleAddrType::Public,
                    ),
                ) {
                    self.fail(i, &e.to_string());
                }
                break;
            }
            if self.links[i].opening && Instant::now() > self.links[i].retry {
                self.fail(i, "连接超时，请唤醒设备");
                let _ = gap.start_scanning(0);
            }
        }
        if self.subscribing.is_none() {
            if let Some((i, h, d)) = self.subscriptions.pop_front() {
                if self.links[i].conn.is_some() {
                    self.links[i].registered.push(h);
                    self.subscribing = Some((i, h, d, Instant::now()));
                    if let Err(e) =
                        self.client
                            .register_for_notify(iface, &BdAddr::from(ADDRESSES[i]), h)
                    {
                        self.fail(i, &e.to_string());
                    }
                }
            }
        }
    }
}

pub fn register(server: &mut esp_idf_svc::http::server::EspHttpServer<'static>) -> Result<()> {
    use esp_idf_svc::{
        http::Method,
        io::{Read, Write},
    };
    server.fn_handler("/api/bluetooth", Method::Get, |req| -> Result<()> {
        let body = {
            let mut s = state().lock().unwrap();
            for d in &mut s.devices {
                d.age_s = d.seen.elapsed().as_secs();
            }
            s.lamp_battery_age_s = s.battery_received.map(|t| t.elapsed().as_secs());
            serde_json::to_vec(&*s)?
        };
        req.into_response(
            200,
            None,
            &[
                ("Content-Type", "application/json"),
                ("Cache-Control", "no-store"),
            ],
        )?
        .write_all(&body)?;
        Ok(())
    })?;
    server.fn_handler("/api/bluetooth", Method::Post, |mut req| -> Result<()> {
        let allowed =
            crate::net::admin_policy::allowed_origin(
                req.header("Origin"),
                req.header("Sec-Fetch-Site"),
                req.header("Host").unwrap_or(""),
            ) && crate::net::admin_auth::authorized(req.header("X-Admin-Key").unwrap_or(""));
        if !allowed {
            req.into_status_response(403)?.write_all(b"Forbidden")?;
            return Ok(());
        }
        let size = req
            .header("Content-Length")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
        if size == 0
            || size > 512
            || req.header("Transfer-Encoding").is_some()
            || req.header("Content-Type") != Some("application/json")
        {
            req.into_status_response(400)?.write_all(b"Invalid body")?;
            return Ok(());
        }
        let mut body = vec![0; size];
        req.read_exact(&mut body)?;
        let command = serde_json::from_slice::<Command>(&body)
            .ok()
            .filter(|c| match c {
                Command::Enable { device, .. } => *device < 2,
                Command::Bind { key, action } => {
                    !key.is_empty()
                        && key.len() <= 384
                        && key
                            .bytes()
                            .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b';')
                        && matches!(action.as_str(), "toggle" | "up" | "down")
                }
                Command::Learn { action } => matches!(action.as_str(), "toggle" | "up" | "down"),
                Command::Lamp { action } => {
                    matches!(action.as_str(), "toggle" | "up" | "down" | "on" | "off")
                }
                _ => true,
            });
        let Some(c) = command else {
            req.into_status_response(400)?
                .write_all(b"Invalid command")?;
            return Ok(());
        };
        if COMMANDS.get().is_some_and(|tx| tx.try_send(c).is_ok()) {
            req.into_response(202, None, &[("Content-Type", "application/json")])?
                .write_all(br#"{"ok":true}"#)?;
        } else {
            req.into_status_response(503)?
                .write_all(b"Bluetooth busy or unavailable")?;
        }
        Ok(())
    })?;
    Ok(())
}
