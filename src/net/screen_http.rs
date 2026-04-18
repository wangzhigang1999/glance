//! 屏幕镜像 HTTP 服务 + 运行时配置 API
//!
//! `GET  /`            → 自刷新 HTML 页(内嵌 `<img src=/screen.bmp>`,1 秒一轮)
//! `GET  /screen.bmp`  → 当前 framebuffer 编码为 1-bit BMP
//! `POST /next`        → 翻页触发
//! `GET  /settings`    → 配置表单页(Tailwind)
//! `GET  /api/config`  → 返回当前 RuntimeConfig(JSON;token 已脱敏)
//! `POST /api/config`  → JSON body,更新字段,保存 NVS
//! `POST /api/reboot`  → esp_restart()

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use esp_idf_svc::http::server::{Configuration, EspHttpServer};
use esp_idf_svc::http::Method;
use esp_idf_svc::io::Write;

use crate::config::{clamp, ConfigStore, SharedConfig};
use crate::display::framebuffer::{pixel_index_mask, BUF_LEN, HEIGHT, WIDTH};

pub type SharedFb = Arc<Mutex<Vec<u8>>>;

pub fn new_shared_fb() -> SharedFb {
    Arc::new(Mutex::new(vec![0xFFu8; BUF_LEN]))
}

const HTML: &str = r##"<!doctype html>
<html lang=en class="h-full">
<head>
<meta charset=utf-8>
<meta name=viewport content="width=device-width,initial-scale=1">
<title>RLCD live</title>
<link rel=preconnect href="https://fonts.googleapis.com">
<link rel=preconnect href="https://fonts.gstatic.com" crossorigin>
<link rel=stylesheet href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=JetBrains+Mono:wght@400;500;600&display=swap">
<link rel=stylesheet href="https://cdnjs.cloudflare.com/ajax/libs/font-awesome/6.5.1/css/all.min.css" crossorigin=anonymous referrerpolicy=no-referrer>
<script src="https://cdn.tailwindcss.com"></script>
<script>
tailwind.config={darkMode:'media',theme:{extend:{
  colors:{
    paper:{DEFAULT:'#f8f7f4',dark:'#12100b'},
    ink:  {DEFAULT:'#252119',dark:'#efeee9'},
    line: {DEFAULT:'#dcd9cf',dark:'#36322a'},
    card: {DEFAULT:'#ffffff',dark:'#1c1911'},
    accent:'#bc3908',
    peach:'#fbd7a5',
  },
  fontFamily:{
    sans:['Inter','-apple-system','system-ui','sans-serif'],
    mono:['"JetBrains Mono"','ui-monospace','Consolas','monospace'],
  }
}}}
</script>
<style type="text/tailwindcss">
@layer components{
  .btn{@apply inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md border border-line dark:border-line-dark text-sm hover:bg-black/5 dark:hover:bg-white/5 transition cursor-pointer whitespace-nowrap}
  .btn-primary{@apply inline-flex items-center gap-1.5 px-3.5 py-1.5 rounded-md bg-accent text-white text-sm font-medium shadow-sm hover:brightness-110 active:translate-y-px transition cursor-pointer whitespace-nowrap}
  .sel{@apply px-2.5 py-1.5 rounded-md border border-line dark:border-line-dark bg-transparent text-sm hover:bg-black/5 dark:hover:bg-white/5 transition cursor-pointer}
  .kbd{@apply ml-1 text-[10px] font-mono bg-black/10 dark:bg-white/10 px-1.5 py-0.5 rounded text-ink/60 dark:text-ink-dark/60}
  .metric b{@apply text-ink dark:text-ink-dark font-semibold}
}
</style>
<style>
  #s{image-rendering:pixelated;aspect-ratio:4/3;height:auto;width:auto;max-height:calc(100dvh - 180px);max-width:calc(100vw - 60px)}
  @media(max-width:640px){
    #s{max-height:calc(100dvh - 130px);max-width:calc(100vw - 24px)}
  }
  .dot-pulse{animation:dp 1.8s ease-in-out infinite}
  @keyframes dp{0%,100%{opacity:1}50%{opacity:.35}}
  /* thin mobile scrollbar on controls overflow */
  .controls-row::-webkit-scrollbar{height:0}
</style>
</head>
<body class="flex flex-col overflow-hidden bg-paper dark:bg-paper-dark text-ink dark:text-ink-dark font-sans antialiased" style="height:100dvh">

<header class="flex items-center gap-2 sm:gap-4 px-3 sm:px-5 py-2 sm:py-2.5 border-b border-line dark:border-line-dark shrink-0 min-w-0">
  <div class="flex items-center gap-2 min-w-0">
    <i class="fa-solid fa-microchip text-accent shrink-0"></i>
    <span class="font-semibold tracking-tight truncate text-sm sm:text-base">
      <span class="hidden sm:inline">ESP32-S3-RLCD-4.2</span>
      <span class="sm:hidden">RLCD-4.2</span>
    </span>
  </div>
  <span id=statusDot class="dot-pulse w-2 h-2 rounded-full bg-emerald-500 shrink-0" aria-hidden=true></span>
  <span id=statusText class="text-[11px] font-mono text-ink/60 dark:text-ink-dark/60 hidden sm:inline">connected</span>
  <div class="ml-auto flex items-center gap-3 sm:gap-4 font-mono text-[11px] sm:text-xs text-ink/60 dark:text-ink-dark/60 metric shrink-0">
    <span class="hidden lg:inline" title="host"><i class="fa-solid fa-globe opacity-60 mr-1"></i><b id=host></b></span>
    <span><i class="fa-solid fa-gauge-high opacity-60 mr-1"></i><b id=fps>-</b><span class="hidden sm:inline"> fps</span></span>
    <span class="hidden sm:inline"><i class="fa-solid fa-film opacity-60 mr-1"></i><b id=frame>0</b></span>
    <span><i class="fa-solid fa-stopwatch opacity-60 mr-1"></i><b id=lat>-</b> ms</span>
  </div>
</header>

<div class="flex items-center gap-2 px-3 sm:px-5 py-2 border-b border-line dark:border-line-dark shrink-0 overflow-x-auto min-w-0">
  <button id=next class=btn-primary>
    <i class="fa-solid fa-forward-step"></i>
    <span class="hidden sm:inline">Next page</span>
    <span class="sm:hidden">Next</span>
    <kbd class="kbd hidden sm:inline">N</kbd>
  </button>
  <button id=pause class=btn aria-label=Pause>
    <i class="fa-solid fa-pause" id=pauseIcon></i>
    <span id=pauseTxt class="hidden sm:inline">Pause</span>
    <kbd class="kbd hidden sm:inline">Space</kbd>
  </button>
  <button id=refresh class=btn aria-label=Refresh>
    <i class="fa-solid fa-rotate-right"></i>
    <span class="hidden sm:inline">Refresh</span>
    <kbd class="kbd hidden sm:inline">R</kbd>
  </button>
  <span class="w-px h-5 bg-line dark:bg-line-dark mx-0.5 sm:mx-1 shrink-0"></span>
  <label for=rate class="text-[10px] uppercase tracking-wider text-ink/50 dark:text-ink-dark/50 shrink-0 hidden sm:inline"><i class="fa-solid fa-clock mr-1"></i>Rate</label>
  <i class="fa-solid fa-clock text-ink/50 dark:text-ink-dark/50 sm:hidden" aria-hidden=true></i>
  <select id=rate class=sel>
    <option value=250>250 ms</option>
    <option value=500>500 ms</option>
    <option value=1000 selected>1 s</option>
    <option value=2000>2 s</option>
    <option value=5000>5 s</option>
  </select>
  <a href="/settings" class="btn ml-auto shrink-0" aria-label=Settings>
    <i class="fa-solid fa-sliders"></i>
    <span class="hidden sm:inline">Settings</span>
  </a>
  <button id=fs class="btn shrink-0" aria-label=Fullscreen>
    <i class="fa-solid fa-expand"></i>
    <span class="hidden sm:inline">Fullscreen</span>
    <kbd class="kbd hidden sm:inline">F</kbd>
  </button>
</div>

<main class="flex-1 min-h-0 flex items-center justify-center p-2 sm:p-4 overflow-hidden">
  <div class="relative p-1.5 sm:p-3 rounded-xl sm:rounded-2xl bg-gradient-to-br from-[#2a2519] to-[#0e0c07] shadow-[0_20px_50px_-10px_rgba(0,0,0,0.45)] ring-1 ring-black/30 flex flex-col max-h-full min-h-0">
    <div class="p-1 sm:p-2 rounded-md sm:rounded-lg bg-black/85 shadow-[inset_0_2px_8px_rgba(0,0,0,0.8)] flex min-h-0">
      <div class="relative rounded overflow-hidden bg-[#e8e4d8] min-h-0">
        <img id=s alt="live screen" class="screen block transition-opacity duration-150">
        <div id=ovl class="hidden absolute inset-0 items-center justify-center bg-black/75 text-accent font-mono text-xs sm:text-sm text-center px-3">
          <i class="fa-solid fa-plug-circle-xmark mr-2"></i>connection lost
        </div>
      </div>
    </div>
    <div class="hidden sm:block text-center text-[9px] font-mono tracking-[3px] text-white/25 mt-2">REFLECTIVE LCD &middot; 400 x 300 &middot; 1-bit</div>
  </div>
</main>

<script>
(()=>{
const $=id=>document.getElementById(id);
const s=$('s'),ovl=$('ovl'),dot=$('statusDot'),stat=$('statusText'),
      next=$('next'),pause=$('pause'),pauseIcon=$('pauseIcon'),pauseTxt=$('pauseTxt'),
      refresh=$('refresh'),rateSel=$('rate'),fs=$('fs'),
      fpsEl=$('fps'),frameEl=$('frame'),latEl=$('lat'),hostEl=$('host');

hostEl.textContent=location.host;
let frames=0,errs=0,paused=false,interval=1000,
    tStart=performance.now(),fpsFrames=0,lastLoad=0,timer=null;

function setStatus(kind,txt){
  dot.className='dot-pulse w-2 h-2 rounded-full '+({
    ok:'bg-emerald-500',err:'bg-accent',idle:'bg-amber-500'
  }[kind]||'bg-emerald-500');
  if(kind!=='ok')dot.classList.remove('dot-pulse');
  stat.textContent=txt||kind;
}
function tick(){
  if(timer){clearTimeout(timer);timer=null}
  if(paused){setStatus('idle','paused');return}
  lastLoad=performance.now();
  s.classList.remove('opacity-50','saturate-50');
  s.src='/screen.bmp?t='+Date.now();
}
s.onload=()=>{
  const now=performance.now();
  latEl.textContent=Math.round(now-lastLoad);
  frames++;fpsFrames++;errs=0;
  frameEl.textContent=frames;
  ovl.classList.remove('flex');ovl.classList.add('hidden');
  setStatus('ok','connected');
  const el=now-tStart;
  if(el>=1000){fpsEl.textContent=(fpsFrames*1000/el).toFixed(1);tStart=now;fpsFrames=0}
  timer=setTimeout(tick,interval);
};
s.onerror=()=>{
  errs++;setStatus('err','error x'+errs);
  if(errs>=2){ovl.classList.remove('hidden');ovl.classList.add('flex');s.classList.add('opacity-50','saturate-50')}
  timer=setTimeout(tick,Math.min(5000,1000+errs*500));
};

next.onclick=async()=>{
  next.disabled=true;next.classList.add('opacity-60');
  try{await fetch('/next',{method:'POST'});setTimeout(()=>{tick();next.disabled=false;next.classList.remove('opacity-60')},180)}
  catch{setStatus('err','next failed');next.disabled=false;next.classList.remove('opacity-60')}
};
pause.onclick=()=>{
  paused=!paused;
  pauseTxt.textContent=paused?'Resume':'Pause';
  pauseIcon.className=paused?'fa-solid fa-play':'fa-solid fa-pause';
  if(!paused)tick();else setStatus('idle','paused');
};
refresh.onclick=()=>{frames=0;frameEl.textContent=0;tick()};
rateSel.onchange=()=>{interval=+rateSel.value;if(!paused)tick()};
fs.onclick=()=>{document.fullscreenElement?document.exitFullscreen():document.documentElement.requestFullscreen()};

addEventListener('keydown',e=>{
  if(['SELECT','INPUT','TEXTAREA'].includes(e.target.tagName))return;
  if(e.code==='KeyN'){e.preventDefault();next.click()}
  else if(e.code==='Space'){e.preventDefault();pause.click()}
  else if(e.code==='KeyR'){e.preventDefault();refresh.click()}
  else if(e.code==='KeyF'){e.preventDefault();fs.click()}
});

tick();
})();
</script>
</body>
</html>
"##;

pub fn start(
    shared: SharedFb,
    next_flag: Arc<AtomicBool>,
    cfg: SharedConfig,
    store: Arc<ConfigStore>,
) -> Result<EspHttpServer<'static>> {
    let srv_cfg = Configuration {
        stack_size: 10 * 1024,
        ..Default::default()
    };
    let mut server = EspHttpServer::new(&srv_cfg)?;

    server.fn_handler("/", Method::Get, |req| -> Result<(), anyhow::Error> {
        let mut resp = req.into_ok_response()?;
        resp.write_all(HTML.as_bytes())?;
        Ok(())
    })?;

    server.fn_handler("/settings", Method::Get, |req| -> Result<(), anyhow::Error> {
        let mut resp = req.into_ok_response()?;
        resp.write_all(SETTINGS_HTML.as_bytes())?;
        Ok(())
    })?;

    let shared_for_handler = shared.clone();
    server.fn_handler(
        "/screen.bmp",
        Method::Get,
        move |req| -> Result<(), anyhow::Error> {
            let fb_copy: Vec<u8> = {
                let guard = shared_for_handler.lock().unwrap();
                guard.clone()
            };
            let bmp = encode_bmp(&fb_copy);
            let len_str = bmp.len().to_string();
            let headers = [
                ("content-type", "image/bmp"),
                ("cache-control", "no-store"),
                ("content-length", len_str.as_str()),
            ];
            let mut resp = req.into_response(200, Some("OK"), &headers)?;
            resp.write_all(&bmp)?;
            Ok(())
        },
    )?;

    let next_for_handler = next_flag.clone();
    server.fn_handler(
        "/next",
        Method::Post,
        move |req| -> Result<(), anyhow::Error> {
            next_for_handler.store(true, Ordering::Relaxed);
            let mut resp = req.into_ok_response()?;
            resp.write_all(b"ok")?;
            Ok(())
        },
    )?;

    // ---- GET /api/config ----
    let cfg_for_get = cfg.clone();
    server.fn_handler(
        "/api/config",
        Method::Get,
        move |req| -> Result<(), anyhow::Error> {
            let json = {
                let c = cfg_for_get.read().unwrap();
                emit_config_json(&c, true)
            };
            let len_str = json.len().to_string();
            let headers = [
                ("content-type", "application/json; charset=utf-8"),
                ("cache-control", "no-store"),
                ("content-length", len_str.as_str()),
            ];
            let mut resp = req.into_response(200, Some("OK"), &headers)?;
            resp.write_all(json.as_bytes())?;
            Ok(())
        },
    )?;

    // ---- POST /api/config ----
    let cfg_for_post = cfg.clone();
    let store_for_post = store.clone();
    server.fn_handler(
        "/api/config",
        Method::Post,
        move |mut req| -> Result<(), anyhow::Error> {
            // 读 body,限制 4KB 防溢出
            let mut buf = [0u8; 4096];
            let mut total = 0usize;
            loop {
                match req.read(&mut buf[total..]) {
                    Ok(0) => break,
                    Ok(n) => {
                        total += n;
                        if total >= buf.len() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let body = std::str::from_utf8(&buf[..total]).unwrap_or("");
            let mut updated = {
                let c = cfg_for_post.read().unwrap();
                c.clone()
            };
            apply_json_patch(&mut updated, body);
            clamp(&mut updated);

            // 持久化
            if let Err(e) = store_for_post.save(&updated) {
                log::warn!("config save failed: {e:#}");
                let mut resp = req.into_status_response(500)?;
                resp.write_all(b"{\"ok\":false,\"error\":\"save failed\"}")?;
                return Ok(());
            }
            // 写回共享
            {
                let mut g = cfg_for_post.write().unwrap();
                *g = updated;
            }
            let json = {
                let c = cfg_for_post.read().unwrap();
                emit_config_json(&c, true)
            };
            let len_str = json.len().to_string();
            let headers = [
                ("content-type", "application/json; charset=utf-8"),
                ("cache-control", "no-store"),
                ("content-length", len_str.as_str()),
            ];
            let mut resp = req.into_response(200, Some("OK"), &headers)?;
            resp.write_all(json.as_bytes())?;
            Ok(())
        },
    )?;

    // ---- POST /api/reboot ----
    server.fn_handler(
        "/api/reboot",
        Method::Post,
        |req| -> Result<(), anyhow::Error> {
            let mut resp = req.into_ok_response()?;
            resp.write_all(b"{\"ok\":true}")?;
            drop(resp);
            // 给 TCP flush 一点时间
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(500));
                unsafe {
                    esp_idf_svc::sys::esp_restart();
                }
            });
            Ok(())
        },
    )?;

    log::info!(
        "Screen HTTP server up on http://<ip>/  (/, /settings, /screen.bmp, /next, /api/config, /api/reboot)"
    );
    Ok(server)
}

fn emit_config_json(c: &crate::config::RuntimeConfig, mask_token: bool) -> String {
    let mut s = String::with_capacity(512);
    s.push('{');
    // 字符串字段
    push_kv_str(&mut s, "gh_user", &c.gh_user);
    s.push(',');
    let token_display = if mask_token && !c.gh_token.is_empty() {
        // 只暴露 "***<后4位>" 给 UI,防窥全值
        let tail = c.gh_token.chars().rev().take(4).collect::<String>();
        let tail: String = tail.chars().rev().collect();
        format!("***{tail}")
    } else {
        c.gh_token.clone()
    };
    push_kv_str(&mut s, "gh_token", &token_display);
    s.push(',');
    push_kv_bool(&mut s, "gh_token_set", !c.gh_token.is_empty());
    s.push(',');
    // 数字字段
    push_kv_u32(&mut s, "contrib_ok_s", c.contrib_ok_s);
    s.push(',');
    push_kv_u32(&mut s, "contrib_err_s", c.contrib_err_s);
    s.push(',');
    push_kv_u32(&mut s, "activity_ok_s", c.activity_ok_s);
    s.push(',');
    push_kv_u32(&mut s, "activity_err_s", c.activity_err_s);
    s.push(',');
    push_kv_u32(&mut s, "activity_stagger_s", c.activity_stagger_s);
    s.push(',');
    push_kv_u32(&mut s, "notif_s", c.notif_s);
    s.push(',');
    push_kv_u32(&mut s, "sensor_refresh_s", c.sensor_refresh_s);
    s.push(',');
    push_kv_bool(&mut s, "auto_rotate", c.auto_rotate);
    s.push(',');
    push_kv_u32(&mut s, "auto_rotate_s", c.auto_rotate_s);
    s.push(',');
    push_kv_f32(&mut s, "temp_off_c", c.temp_off_c);
    s.push(',');
    push_kv_f32(&mut s, "humid_off_pct", c.humid_off_pct);
    s.push(',');
    push_kv_i32(&mut s, "tz_off_s", c.tz_off_s);
    s.push(',');
    push_kv_u32(&mut s, "splash_flash", c.splash_flash);
    s.push('}');
    s
}

fn push_kv_str(out: &mut String, k: &str, v: &str) {
    out.push('"');
    out.push_str(k);
    out.push_str("\":\"");
    for c in v.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push('?'),
            c => out.push(c),
        }
    }
    out.push('"');
}
fn push_kv_u32(out: &mut String, k: &str, v: u32) {
    out.push('"');
    out.push_str(k);
    out.push_str("\":");
    out.push_str(&v.to_string());
}
fn push_kv_i32(out: &mut String, k: &str, v: i32) {
    out.push('"');
    out.push_str(k);
    out.push_str("\":");
    out.push_str(&v.to_string());
}
fn push_kv_f32(out: &mut String, k: &str, v: f32) {
    out.push('"');
    out.push_str(k);
    out.push_str("\":");
    out.push_str(&format!("{:.2}", v));
}
fn push_kv_bool(out: &mut String, k: &str, v: bool) {
    out.push('"');
    out.push_str(k);
    out.push_str("\":");
    out.push_str(if v { "true" } else { "false" });
}

/// 从 JSON body 抽出字段更新。未出现的字段保留原值。脱敏 token 串("***xxxx" 开头)会被忽略。
fn apply_json_patch(c: &mut crate::config::RuntimeConfig, body: &str) {
    if let Some(v) = find_str(body, "gh_user") {
        c.gh_user = v;
    }
    if let Some(v) = find_str(body, "gh_token") {
        // 前端提交脱敏占位则不覆盖
        if !v.starts_with("***") && !v.is_empty() {
            c.gh_token = v;
        } else if v.is_empty() {
            // 显式清空(用户留空 token 字段)
            c.gh_token.clear();
        }
    }
    if let Some(v) = find_u32(body, "contrib_ok_s") {
        c.contrib_ok_s = v;
    }
    if let Some(v) = find_u32(body, "contrib_err_s") {
        c.contrib_err_s = v;
    }
    if let Some(v) = find_u32(body, "activity_ok_s") {
        c.activity_ok_s = v;
    }
    if let Some(v) = find_u32(body, "activity_err_s") {
        c.activity_err_s = v;
    }
    if let Some(v) = find_u32(body, "activity_stagger_s") {
        c.activity_stagger_s = v;
    }
    if let Some(v) = find_u32(body, "notif_s") {
        c.notif_s = v;
    }
    if let Some(v) = find_u32(body, "sensor_refresh_s") {
        c.sensor_refresh_s = v;
    }
    if let Some(v) = find_bool(body, "auto_rotate") {
        c.auto_rotate = v;
    }
    if let Some(v) = find_u32(body, "auto_rotate_s") {
        c.auto_rotate_s = v;
    }
    if let Some(v) = find_f32(body, "temp_off_c") {
        c.temp_off_c = v;
    }
    if let Some(v) = find_f32(body, "humid_off_pct") {
        c.humid_off_pct = v;
    }
    if let Some(v) = find_i32(body, "tz_off_s") {
        c.tz_off_s = v;
    }
    if let Some(v) = find_u32(body, "splash_flash") {
        c.splash_flash = v;
    }
}

fn find_key_pos(body: &str, key: &str) -> Option<usize> {
    // 匹配 "key":
    let needle = format!("\"{}\"", key);
    let mut cursor = 0;
    while let Some(rel) = body[cursor..].find(&needle) {
        let after = cursor + rel + needle.len();
        let rest = &body[after..];
        let trimmed = rest.trim_start();
        if trimmed.starts_with(':') {
            return Some(after + (rest.len() - trimmed.len()) + 1);
        }
        cursor = after;
    }
    None
}
fn find_str(body: &str, key: &str) -> Option<String> {
    let p = find_key_pos(body, key)?;
    let rest = body[p..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut it = rest.chars();
    while let Some(ch) = it.next() {
        if ch == '\\' {
            match it.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                o => out.push(o),
            }
        } else if ch == '"' {
            return Some(out);
        } else {
            out.push(ch);
        }
    }
    None
}
fn find_num_raw<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let p = find_key_pos(body, key)?;
    let rest = body[p..].trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '-' || c == '.' || c == '+'))
        .unwrap_or(rest.len());
    if end == 0 {
        None
    } else {
        // 把相对 rest 的切片映射回 body 的切片
        let start_in_body = body.len() - rest.len();
        Some(&body[start_in_body..start_in_body + end])
    }
}
fn find_u32(body: &str, key: &str) -> Option<u32> {
    find_num_raw(body, key)?.parse().ok()
}
fn find_i32(body: &str, key: &str) -> Option<i32> {
    find_num_raw(body, key)?.parse().ok()
}
fn find_f32(body: &str, key: &str) -> Option<f32> {
    find_num_raw(body, key)?.parse().ok()
}
fn find_bool(body: &str, key: &str) -> Option<bool> {
    let p = find_key_pos(body, key)?;
    let rest = body[p..].trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

// ============================================================================
// Settings 页 HTML
// ============================================================================
const SETTINGS_HTML: &str = r##"<!doctype html>
<html lang=en class="h-full">
<head>
<meta charset=utf-8>
<meta name=viewport content="width=device-width,initial-scale=1">
<title>RLCD settings</title>
<link rel=preconnect href="https://fonts.googleapis.com">
<link rel=preconnect href="https://fonts.gstatic.com" crossorigin>
<link rel=stylesheet href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=JetBrains+Mono:wght@400;500;600&display=swap">
<link rel=stylesheet href="https://cdnjs.cloudflare.com/ajax/libs/font-awesome/6.5.1/css/all.min.css" crossorigin=anonymous referrerpolicy=no-referrer>
<script src="https://cdn.tailwindcss.com"></script>
<script>
tailwind.config={darkMode:'media',theme:{extend:{
  colors:{
    paper:{DEFAULT:'#f8f7f4',dark:'#12100b'},
    ink:  {DEFAULT:'#252119',dark:'#efeee9'},
    line: {DEFAULT:'#dcd9cf',dark:'#36322a'},
    card: {DEFAULT:'#ffffff',dark:'#1c1911'},
    accent:'#bc3908',
    peach:'#fbd7a5',
  },
  fontFamily:{
    sans:['Inter','-apple-system','system-ui','sans-serif'],
    mono:['"JetBrains Mono"','ui-monospace','Consolas','monospace'],
  }
}}}
</script>
<style type="text/tailwindcss">
@layer components{
  .input{@apply w-full px-3 py-2 rounded-md border border-line dark:border-line-dark bg-card dark:bg-card-dark text-sm font-mono focus:outline-none focus:ring-2 focus:ring-accent/40 focus:border-accent/60 transition}
  .label{@apply block text-[11px] uppercase tracking-wider text-ink/60 dark:text-ink-dark/60 mb-1.5 font-sans}
  .hint{@apply text-[11px] text-ink/50 dark:text-ink-dark/50 mt-1 font-sans}
  .card{@apply rounded-xl border border-line dark:border-line-dark bg-card dark:bg-card-dark p-5 sm:p-6 shadow-sm}
  .card h2{@apply flex items-center gap-2 text-sm font-semibold tracking-tight uppercase text-ink/80 dark:text-ink-dark/80 mb-4}
  .card h2 i{@apply text-accent}
  .btn{@apply inline-flex items-center gap-2 px-4 py-2 rounded-md border border-line dark:border-line-dark text-sm hover:bg-black/5 dark:hover:bg-white/5 transition cursor-pointer}
  .btn-primary{@apply inline-flex items-center gap-2 px-4 py-2 rounded-md bg-accent text-white text-sm font-medium shadow-sm hover:brightness-110 active:translate-y-px transition cursor-pointer}
  .btn-danger{@apply inline-flex items-center gap-2 px-4 py-2 rounded-md border border-accent/40 text-accent text-sm hover:bg-accent/10 transition cursor-pointer}
}
</style>
</head>
<body class="min-h-full bg-paper dark:bg-paper-dark text-ink dark:text-ink-dark font-sans antialiased">

<header class="sticky top-0 z-10 flex items-center gap-3 px-4 sm:px-6 py-3 border-b border-line dark:border-line-dark bg-paper/80 dark:bg-paper-dark/80 backdrop-blur">
  <a href="/" class="btn !px-3 !py-1.5 shrink-0"><i class="fa-solid fa-arrow-left"></i><span class="hidden sm:inline">Live</span></a>
  <div class="flex items-center gap-2 min-w-0">
    <i class="fa-solid fa-sliders text-accent"></i>
    <span class="font-semibold tracking-tight truncate">Settings</span>
    <span class="hidden sm:inline text-[11px] font-mono text-ink/50 dark:text-ink-dark/50">runtime config</span>
  </div>
  <div class="ml-auto flex items-center gap-2">
    <span id=toast class="hidden sm:inline text-[11px] font-mono text-ink/50 dark:text-ink-dark/50"></span>
  </div>
</header>

<main class="max-w-3xl mx-auto px-4 sm:px-6 py-6 space-y-5">

  <section class=card>
    <h2><i class="fa-brands fa-github"></i>GitHub identity</h2>
    <div class="grid grid-cols-1 sm:grid-cols-2 gap-4">
      <div>
        <label class=label for=gh_user>Username</label>
        <input class=input id=gh_user placeholder="wangzhigang1999">
        <div class=hint>Your GitHub login. Fetchers read on next cycle.</div>
      </div>
      <div>
        <label class=label for=gh_token>Personal access token</label>
        <input class=input id=gh_token type=password autocomplete=off placeholder="ghp_...">
        <div class=hint>Scope: <code>notifications</code> + <code>repo</code>. Shown masked; leave unchanged to keep.</div>
      </div>
    </div>
  </section>

  <section class=card>
    <h2><i class="fa-solid fa-clock-rotate-left"></i>Fetcher intervals (seconds)</h2>
    <div class="grid grid-cols-2 sm:grid-cols-3 gap-4">
      <div><label class=label>Contribution ok</label><input class=input type=number min=30 max=86400 id=contrib_ok_s></div>
      <div><label class=label>Contribution err</label><input class=input type=number min=30 max=86400 id=contrib_err_s></div>
      <div><label class=label>Notifications</label><input class=input type=number min=30 max=86400 id=notif_s></div>
      <div><label class=label>Activity ok</label><input class=input type=number min=30 max=86400 id=activity_ok_s></div>
      <div><label class=label>Activity err</label><input class=input type=number min=30 max=86400 id=activity_err_s></div>
      <div><label class=label>Activity stagger</label><input class=input type=number min=0 max=600 id=activity_stagger_s></div>
    </div>
    <div class="hint mt-3">Shorter = more fresh, more GitHub API quota used. Defaults: contrib 300/120, activity 180/120, notif 180.</div>
  </section>

  <section class=card>
    <h2><i class="fa-solid fa-display"></i>Display loop</h2>
    <div class="grid grid-cols-2 sm:grid-cols-3 gap-4">
      <div><label class=label>Sensor refresh (s)</label><input class=input type=number min=1 max=3600 id=sensor_refresh_s></div>
      <div class="flex items-end"><label class="inline-flex items-center gap-2 cursor-pointer select-none"><input type=checkbox id=auto_rotate class="w-4 h-4 accent-accent"><span class="text-sm">Auto rotate pages</span></label></div>
      <div><label class=label>Rotate period (s)</label><input class=input type=number min=3 max=3600 id=auto_rotate_s></div>
      <div><label class=label>Splash flash cycles</label><input class=input type=number min=0 max=64 id=splash_flash>
        <div class=hint>Boot-only; applies on next restart.</div>
      </div>
    </div>
  </section>

  <section class=card>
    <h2><i class="fa-solid fa-temperature-half"></i>Sensor calibration & timezone</h2>
    <div class="grid grid-cols-2 sm:grid-cols-3 gap-4">
      <div><label class=label>Temp offset (&deg;C)</label><input class=input type=number step=0.1 min=-20 max=20 id=temp_off_c></div>
      <div><label class=label>Humidity offset (%)</label><input class=input type=number step=0.1 min=-50 max=50 id=humid_off_pct></div>
      <div><label class=label>TZ offset (seconds)</label><input class=input type=number min=-50400 max=50400 id=tz_off_s>
        <div class=hint>28800 = UTC+8 (CN).</div>
      </div>
    </div>
  </section>

  <div class="flex flex-wrap gap-3 items-center pt-2 sticky bottom-3 bg-paper/80 dark:bg-paper-dark/80 backdrop-blur border border-line dark:border-line-dark rounded-xl p-3">
    <button class=btn-primary id=save><i class="fa-solid fa-floppy-disk"></i>Save</button>
    <button class=btn id=reload><i class="fa-solid fa-rotate-right"></i>Reload</button>
    <button class=btn-danger id=reboot><i class="fa-solid fa-power-off"></i>Reboot device</button>
    <span class="text-[11px] font-mono text-ink/50 dark:text-ink-dark/50 ml-auto">values apply on next fetcher cycle</span>
  </div>

</main>

<script>
(()=>{
const $=id=>document.getElementById(id);
const TXT_FIELDS=['gh_user','gh_token'];
const BOOL_FIELDS=['auto_rotate'];
const NUM_FIELDS=['contrib_ok_s','contrib_err_s','activity_ok_s','activity_err_s','activity_stagger_s','notif_s','sensor_refresh_s','auto_rotate_s','temp_off_c','humid_off_pct','tz_off_s','splash_flash'];
const toast=$('toast');

function showToast(msg,kind){
  toast.textContent=msg;toast.className='text-[11px] font-mono '+(kind==='err'?'text-accent':'text-emerald-600 dark:text-emerald-400');
  toast.classList.remove('hidden');
  clearTimeout(showToast._t);showToast._t=setTimeout(()=>toast.classList.add('hidden'),3500);
}

let originalToken='';
async function load(){
  try{
    const r=await fetch('/api/config',{cache:'no-store'});
    const c=await r.json();
    TXT_FIELDS.forEach(k=>{if(k in c)$(k).value=c[k]});
    BOOL_FIELDS.forEach(k=>{if(k in c)$(k).checked=!!c[k]});
    NUM_FIELDS.forEach(k=>{if(k in c)$(k).value=c[k]});
    originalToken=c.gh_token||'';
    showToast('loaded','ok');
  }catch(e){showToast('load failed','err')}
}
async function save(){
  const payload={};
  TXT_FIELDS.forEach(k=>{
    const v=$(k).value;
    // token 未修改则不提交,避免把脱敏占位写回
    if(k==='gh_token'&&v===originalToken)return;
    payload[k]=v;
  });
  BOOL_FIELDS.forEach(k=>payload[k]=$(k).checked);
  NUM_FIELDS.forEach(k=>{
    const v=$(k).value;
    if(v==='')return;
    payload[k]=(k==='temp_off_c'||k==='humid_off_pct')?parseFloat(v):parseInt(v,10);
  });
  try{
    const r=await fetch('/api/config',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(payload)});
    if(!r.ok)throw new Error('HTTP '+r.status);
    const c=await r.json();
    originalToken=c.gh_token||'';
    $('gh_token').value=originalToken;
    showToast('saved','ok');
  }catch(e){showToast('save failed: '+e.message,'err')}
}
async function reboot(){
  if(!confirm('Reboot the device? Display will flash.'))return;
  try{await fetch('/api/reboot',{method:'POST'});showToast('rebooting...','ok')}
  catch{showToast('reboot failed','err')}
}
$('save').onclick=save;
$('reload').onclick=load;
$('reboot').onclick=reboot;
load();
})();
</script>
</body>
</html>
"##;

/// 把 ST7305 本地 fb 编码成标准 1-bit BMP。
fn encode_bmp(fb: &[u8]) -> Vec<u8> {
    // BMP 1-bit:每行字节 = ((W*1 + 31)/32) * 4。W=400 → 52.
    const W: usize = WIDTH as usize;
    const H: usize = HEIGHT as usize;
    const ROW: usize = ((W + 31) / 32) * 4; // 52
    const PIXEL_DATA_LEN: usize = ROW * H;  // 15600

    const FILE_HDR: usize = 14;
    const DIB_HDR: usize = 40;
    const PALETTE: usize = 8; // 2 entries × 4 bytes
    const OFFSET: usize = FILE_HDR + DIB_HDR + PALETTE; // 62
    const FILE_SIZE: usize = OFFSET + PIXEL_DATA_LEN; // 15662

    let mut out: Vec<u8> = Vec::with_capacity(FILE_SIZE);

    // ---- BITMAPFILEHEADER ----
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(FILE_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());           // reserved
    out.extend_from_slice(&(OFFSET as u32).to_le_bytes());

    // ---- BITMAPINFOHEADER ----
    out.extend_from_slice(&(DIB_HDR as u32).to_le_bytes());
    out.extend_from_slice(&(W as i32).to_le_bytes());
    // 负 height => top-down
    out.extend_from_slice(&(-(H as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());           // planes
    out.extend_from_slice(&1u16.to_le_bytes());           // bpp
    out.extend_from_slice(&0u32.to_le_bytes());           // BI_RGB
    out.extend_from_slice(&(PIXEL_DATA_LEN as u32).to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());        // x ppm
    out.extend_from_slice(&2835u32.to_le_bytes());        // y ppm
    out.extend_from_slice(&2u32.to_le_bytes());           // palette entries
    out.extend_from_slice(&0u32.to_le_bytes());           // important

    // ---- palette: [0]=white, [1]=black ----
    out.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0x00]);
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);

    // ---- pixel data ----
    // 每行 ROW 字节,BMP MSB-first。bit=1 表示黑(palette[1])。
    // fb bit=0 表示前景(黑)。我们要把 "黑" 映射到 bit=1。
    let row_start = out.len();
    out.resize(row_start + PIXEL_DATA_LEN, 0);
    for y in 0..H {
        let out_row = &mut out[row_start + y * ROW..row_start + y * ROW + ROW];
        for x_byte in 0..(W / 8) {
            let mut b: u8 = 0;
            for bit in 0..8 {
                let x = x_byte * 8 + bit;
                let (idx, mask) = pixel_index_mask(x as u16, y as u16);
                // fb 前景 bit=0 → 显示黑 → BMP bit=1
                let fg = (fb[idx] & mask) == 0;
                if fg {
                    b |= 1 << (7 - bit);
                }
            }
            out_row[x_byte] = b;
        }
        // 剩余 pad 字节保持 0
    }

    out
}
