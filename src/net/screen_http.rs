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

const HTML: &str = r##"<!doctype html><meta charset=utf-8>
<title>RLCD live</title>
<style>
  html,body{background:#1b1b1b;color:#ddd;font-family:system-ui,sans-serif;margin:0;padding:12px}
  img{image-rendering:pixelated;width:800px;height:600px;border:2px solid #444;background:#fff;display:block;margin-top:10px}
  .row{display:flex;gap:16px;align-items:center;margin-bottom:10px;flex-wrap:wrap}
  button{background:#333;color:#ddd;border:1px solid #555;padding:6px 14px;cursor:pointer;font-size:14px;border-radius:4px}
  button:hover{background:#444}
  button:active{background:#222}
</style>
<div class=row>
  <h3 style=margin:0>ESP32-S3-RLCD-4.2 live screen</h3>
  <button id=next>next page (KEY)</button>
  <span id=stat style=color:#8c8></span>
</div>
<img id=s>
<script>
  const s=document.getElementById('s'),stat=document.getElementById('stat'),btn=document.getElementById('next');
  let n=0,err=0;
  function r(){s.src='/screen.bmp?t='+Date.now();}
  s.onload=()=>{n++;err=0;stat.textContent='frame '+n;setTimeout(r,1000);}
  s.onerror=()=>{err++;stat.textContent='err '+err;setTimeout(r,2000);}
  btn.onclick=async()=>{
    try{await fetch('/next',{method:'POST'});setTimeout(r,200);}
    catch(e){stat.textContent='next err';}
  };
  r();
</script>
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
