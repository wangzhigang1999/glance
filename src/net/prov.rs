//! BLE 配网(custom GATT)
//!
//! GATT 设计:
//! ```text
//!   Service  524c4344-c001-4c7c-9b4f-000000000000  ("RLCD"+...)
//!     ├── SSID     ...001  WRITE   UTF-8 <=32B
//!     ├── PASSWORD ...002  WRITE   UTF-8 <=64B
//!     ├── COMMIT   ...003  WRITE   u8 0x01 = "try connect"
//!     └── STATUS   ...004  READ + NOTIFY   u8 枚举 ProvStatus
//! ```
//!
//! 手机侧(nRF Connect / 自定义 App):
//! 1. 扫到 "RLCD-Thermo" → 连接
//! 2. 写 SSID(UTF-8 字节)
//! 3. 写 PASSWORD
//! 4. 写 COMMIT = 0x01
//! 5. 订阅 STATUS 的 notify,观察 Received→Connecting→Connected/Failed
//!
//! 主线程通过 `try_recv_creds` 取回 SSID+PWD,BLE 回调只负责凑齐 + 发信号。

use anyhow::{Context, Result};
use esp32_nimble::{
    utilities::mutex::Mutex as NimMutex, uuid128, BLEAdvertisementData, BLECharacteristic,
    BLEDevice, NimbleProperties,
};
use std::sync::{
    mpsc::{self, Receiver, Sender, TryRecvError},
    Arc, Mutex,
};

use super::wifi::WifiCreds;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ProvStatus {
    Idle = 0,
    Received = 1,
    Connecting = 2,
    Connected = 3,
    Failed = 4,
}

#[derive(Default)]
struct ProvInner {
    ssid: String,
    password: String,
}

pub struct Provisioner {
    creds_rx: Receiver<WifiCreds>,
    status_char: Arc<NimMutex<BLECharacteristic>>,
}

impl Provisioner {
    /// 启动 BLE 广播 + GATT,立刻返回。创建之后 BLE 栈一直跑,
    /// 直到 `shutdown()` 被调用(释放 ~30KB RAM 给 WiFi 用)。
    pub fn start(device_name: &str) -> Result<Self> {
        let device = BLEDevice::take();
        let server = device.get_server();

        server.on_connect(|_srv, desc| {
            log::info!("BLE client connected: {:?}", desc);
        });
        server.on_disconnect(|_desc, reason| {
            log::info!("BLE client disconnected: {:?}", reason);
        });

        let service = server.create_service(uuid128!("524c4344-c001-4c7c-9b4f-000000000000"));

        let ssid_char = service.lock().create_characteristic(
            uuid128!("524c4344-c001-4c7c-9b4f-000000000001"),
            NimbleProperties::WRITE,
        );
        let pwd_char = service.lock().create_characteristic(
            uuid128!("524c4344-c001-4c7c-9b4f-000000000002"),
            NimbleProperties::WRITE,
        );
        let commit_char = service.lock().create_characteristic(
            uuid128!("524c4344-c001-4c7c-9b4f-000000000003"),
            NimbleProperties::WRITE,
        );
        let status_char = service.lock().create_characteristic(
            uuid128!("524c4344-c001-4c7c-9b4f-000000000004"),
            NimbleProperties::READ | NimbleProperties::NOTIFY,
        );
        status_char.lock().set_value(&[ProvStatus::Idle as u8]);

        let shared = Arc::new(Mutex::new(ProvInner::default()));
        let (tx, rx): (Sender<WifiCreds>, Receiver<WifiCreds>) = mpsc::channel();

        // SSID 写回调:缓存进 shared
        {
            let shared = shared.clone();
            ssid_char.lock().on_write(move |args| {
                let bytes = args.recv_data();
                let s = String::from_utf8_lossy(bytes).into_owned();
                log::info!("BLE prov: SSID recv ({} bytes)", s.len());
                if let Ok(mut inner) = shared.lock() {
                    inner.ssid = s;
                }
            });
        }

        // PASSWORD 写回调
        {
            let shared = shared.clone();
            pwd_char.lock().on_write(move |args| {
                let bytes = args.recv_data();
                let s = String::from_utf8_lossy(bytes).into_owned();
                log::info!("BLE prov: PASSWORD recv ({} bytes)", s.len());
                if let Ok(mut inner) = shared.lock() {
                    inner.password = s;
                }
            });
        }

        // COMMIT 写回调:组 WifiCreds,推给 mpsc,主线程接住去连 WiFi
        {
            let shared = shared.clone();
            let status_char = status_char.clone();
            commit_char.lock().on_write(move |args| {
                let data = args.recv_data();
                if data.first().copied() != Some(0x01) {
                    log::warn!("BLE prov: commit != 0x01, ignored: {:?}", data);
                    return;
                }
                let (ssid, password) = {
                    let inner = match shared.lock() {
                        Ok(g) => g,
                        Err(_) => return,
                    };
                    (inner.ssid.clone(), inner.password.clone())
                };
                if ssid.is_empty() {
                    log::warn!("BLE prov: commit but SSID empty");
                    status_char.lock().set_value(&[ProvStatus::Failed as u8]).notify();
                    return;
                }
                match WifiCreds::new(&ssid, &password) {
                    Ok(creds) => {
                        log::info!("BLE prov: commit OK, ssid={ssid}");
                        status_char
                            .lock()
                            .set_value(&[ProvStatus::Received as u8])
                            .notify();
                        let _ = tx.send(creds);
                    }
                    Err(e) => {
                        log::error!("BLE prov: invalid creds: {e}");
                        status_char
                            .lock()
                            .set_value(&[ProvStatus::Failed as u8])
                            .notify();
                    }
                }
            });
        }

        let advertising = device.get_advertising();
        advertising
            .lock()
            .set_data(
                BLEAdvertisementData::new()
                    .name(device_name)
                    .add_service_uuid(uuid128!("524c4344-c001-4c7c-9b4f-000000000000")),
            )
            .context("set adv data")?;
        advertising.lock().start().context("adv start")?;

        log::info!("BLE provisioning started, advertising as '{device_name}'");
        Ok(Self {
            creds_rx: rx,
            status_char,
        })
    }

    pub fn try_recv_creds(&self) -> Option<WifiCreds> {
        match self.creds_rx.try_recv() {
            Ok(c) => Some(c),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => None,
        }
    }

    /// 阻塞等凭据。超时返回 None(由调用方决定是否继续)
    pub fn wait_for_creds(&self, timeout: std::time::Duration) -> Option<WifiCreds> {
        self.creds_rx.recv_timeout(timeout).ok()
    }

    pub fn publish_status(&self, status: ProvStatus) {
        self.status_char
            .lock()
            .set_value(&[status as u8])
            .notify();
        log::info!("BLE prov status -> {:?}", status);
    }

    /// 配网完成后关 BLE,释放 ~30KB 给 WiFi。
    /// 注:deinit 后不能再 take BLEDevice,整次启动就不再用 BLE。
    pub fn shutdown() -> Result<()> {
        BLEDevice::deinit().context("BLEDevice::deinit")?;
        log::info!("BLE stack deinitialized");
        Ok(())
    }
}
