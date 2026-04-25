//! 日志分流:wrap EspLogger,把每条 Rust `log!` 同时送到串口 + 内存 ringbuffer
//!
//! 仅捕获 Rust 侧 `log` crate 的调用;ESP-IDF C 组件(wifi/lwip/httpd 等直接调
//! `ESP_LOG*`)的日志只到串口、不进 ringbuffer。后者要捕获得换 `esp_log_set_vprintf` 钩子。
//!
//! 不用 SSE 是因为 esp-idf httpd 单 task 处理所有 handler,长连接会卡死整个 server。
//! 改成 ringbuffer + 单调递增 seq + `/logs.json?since=N` 短轮询,客户端按 seq 拉增量。
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use esp_idf_svc::log::{EspIdfLogFilter, EspLogger};
use log::{Log, Metadata, Record};

/// ringbuffer 容量:超过就丢最老的(轮询客户端落后太多会漏一段,前端可见 next_seq 跳跃)
const HISTORY_CAP: usize = 400;
/// 单条日志超出就裁剪(防 args!() 喷一坨)
const LINE_MAX: usize = 512;

struct State {
    /// 下次新行用的 seq;同时也等于"当前已分配过的最大 seq + 1"
    next_seq: u64,
    /// (seq, line) FIFO,容量 HISTORY_CAP
    history: VecDeque<(u64, String)>,
}

/// 给 HTTP 端用的句柄。
pub struct LogHub {
    state: Arc<Mutex<State>>,
}

impl LogHub {
    /// 拉取 seq 严格大于 `since` 的所有行,返回 `(next_seq, lines)`。
    /// 客户端把返回的 `next_seq` 当下次 `since` 用即可。
    /// 若 `since` 比 ringbuffer 头还旧,漏掉的中间条无法恢复(前端可对比相邻 next_seq 跳幅判断丢了多少)。
    pub fn since(&self, since: u64) -> (u64, Vec<String>) {
        let s = self.state.lock().unwrap();
        let lines = s
            .history
            .iter()
            .filter(|(seq, _)| *seq >= since)
            .map(|(_, l)| l.clone())
            .collect();
        (s.next_seq, lines)
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

        // 2) 自己再格式化一份简短行给 ringbuffer
        let ts_ms = unsafe { esp_idf_svc::sys::esp_log_timestamp() };
        let mut line = format!(
            "{:>8} {} {}: {}",
            ts_ms,
            level_marker(record.level()),
            record.metadata().target(),
            record.args()
        );
        truncate_utf8(&mut line, LINE_MAX);

        // 3) 推 history。锁内只 push,不做 IO。
        let Ok(mut s) = self.state.lock() else {
            return;
        };
        let seq = s.next_seq;
        s.next_seq = s.next_seq.wrapping_add(1);
        s.history.push_back((seq, line));
        while s.history.len() > HISTORY_CAP {
            s.history.pop_front();
        }
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
        next_seq: 1, // 0 留给客户端首次请求做 "give me everything"
        history: VecDeque::with_capacity(HISTORY_CAP),
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
