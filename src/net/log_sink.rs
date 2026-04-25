//! 日志分流:wrap EspLogger,把每条 Rust `log!` 同时送到串口 + 内存 ringbuffer + SSE 订阅者
//!
//! 仅捕获 Rust 侧 `log` crate 的调用;ESP-IDF C 组件(wifi/lwip/httpd 等直接调
//! `ESP_LOG*`)的日志只到串口、不进 ringbuffer。后者要捕获得换 `esp_log_set_vprintf` 钩子。
//!
//! 用法:
//! ```ignore
//! let hub = log_sink::install();          // 替代 EspLogger::initialize_default()
//! let snapshot = hub.snapshot();          // 取 history
//! let rx = hub.subscribe();               // SSE 订阅一条新日志流
//! ```
use std::{
    collections::VecDeque,
    sync::{
        mpsc::{sync_channel, Receiver, SyncSender, TrySendError},
        Arc, Mutex,
    },
};

use esp_idf_svc::log::{EspIdfLogFilter, EspLogger};
use log::{Log, Metadata, Record};

/// ringbuffer 容量:新连接进来先重放这么多条历史
const HISTORY_CAP: usize = 400;
/// 单条日志超出就裁剪(防 args!() 喷一坨)
const LINE_MAX: usize = 512;
/// 每订阅者的缓冲条数:慢客户端跟不上时丢新条但保连接
const SUB_CHAN_CAP: usize = 64;

struct State {
    history: VecDeque<String>,
    subs: Vec<SyncSender<String>>,
}

/// 给 HTTP 端用的句柄。
pub struct LogHub {
    state: Arc<Mutex<State>>,
}

impl LogHub {
    /// 拷贝一份当前 history(给新连接重放历史)。
    pub fn snapshot(&self) -> Vec<String> {
        self.state.lock().unwrap().history.iter().cloned().collect()
    }

    /// 订阅后续日志。channel 满会丢新条(保连接)、断线时 logger 端会自动剔除。
    pub fn subscribe(&self) -> Receiver<String> {
        let (tx, rx) = sync_channel(SUB_CHAN_CAP);
        self.state.lock().unwrap().subs.push(tx);
        rx
    }
}

struct Sink {
    inner: EspLogger,
    state: Arc<Mutex<State>>,
}

impl Log for Sink {
    fn enabled(&self, m: &Metadata) -> bool {
        self.inner.enabled(m)
    }

    fn log(&self, record: &Record) {
        // 1) 转发到串口(沿用 ESP-IDF 格式 + 颜色 + 等级过滤)
        self.inner.log(record);
        if !self.inner.enabled(record.metadata()) {
            return;
        }

        // 2) 自己再格式化一份简短行给 ringbuffer / SSE
        let ts_ms = unsafe { esp_idf_svc::sys::esp_log_timestamp() };
        let mut line = format!(
            "{:>8} {} {}: {}",
            ts_ms,
            level_marker(record.level()),
            record.metadata().target(),
            record.args()
        );
        truncate_utf8(&mut line, LINE_MAX);

        // 3) 推 history + 广播。锁内不做 IO,只 push 队列。
        let Ok(mut s) = self.state.lock() else {
            return;
        };
        if s.history.len() >= HISTORY_CAP {
            s.history.pop_front();
        }
        s.history.push_back(line.clone());
        s.subs.retain(|tx| match tx.try_send(line.clone()) {
            Ok(()) => true,
            // 慢订阅者:丢这条但保留连接(下次再试)
            Err(TrySendError::Full(_)) => true,
            // 接收端已 drop(SSE handler 退出):剔除
            Err(TrySendError::Disconnected(_)) => false,
        });
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

/// 替代 `EspLogger::initialize_default()`。返回的 hub 给 HTTP 端用。
///
/// 只能调用一次(`set_boxed_logger` 全局唯一)。
pub fn install() -> Arc<LogHub> {
    let inner = EspLogger::new(EspIdfLogFilter::new());
    // 把 log crate 的 max_level 设成 ESP-IDF 配置值(对齐 initialize_default 的行为)
    log::set_max_level(inner.filter().get_max_level());

    let state = Arc::new(Mutex::new(State {
        history: VecDeque::with_capacity(HISTORY_CAP),
        subs: Vec::new(),
    }));
    let sink = Sink {
        inner,
        state: state.clone(),
    };
    log::set_boxed_logger(Box::new(sink)).expect("log_sink::install called twice");
    Arc::new(LogHub { state })
}

fn level_marker(l: log::Level) -> &'static str {
    match l {
        log::Level::Error => "E",
        log::Level::Warn => "W",
        log::Level::Info => "I",
        log::Level::Debug => "D",
        log::Level::Trace => "V",
    }
}

/// 按 UTF-8 char boundary 截断,防 `String::truncate` 在多字节中间切到 panic。
fn truncate_utf8(s: &mut String, max: usize) {
    if s.len() <= max {
        return;
    }
    let mut cut = max;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s.truncate(cut);
}
