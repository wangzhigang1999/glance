//! 屏幕镜像 HTTP 服务
//!
//! `GET /`           → 自刷新 HTML 页(内嵌 `<img src=/screen.bmp>`,1 秒一轮)
//! `GET /screen.bmp` → 当前 framebuffer 编码为 1-bit BMP
//!
//! 固件端不做 WebSocket / push,浏览器端轮询 1s 足够观察 UI 变化。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use esp_idf_svc::http::server::{Configuration, EspHttpServer};
use esp_idf_svc::http::Method;
use esp_idf_svc::io::Write;

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
  <button id=fs class="btn ml-auto shrink-0" aria-label=Fullscreen>
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
) -> Result<EspHttpServer<'static>> {
    let config = Configuration {
        stack_size: 8 * 1024,
        ..Default::default()
    };
    let mut server = EspHttpServer::new(&config)?;

    server.fn_handler("/", Method::Get, |req| -> Result<(), anyhow::Error> {
        let mut resp = req.into_ok_response()?;
        resp.write_all(HTML.as_bytes())?;
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

    log::info!("Screen HTTP server up on http://<ip>/ (BMP at /screen.bmp, POST /next cycles page)");
    Ok(server)
}

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
