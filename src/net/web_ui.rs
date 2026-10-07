//! Build-time React assets. Stream gzip bytes from flash; never decompress into device RAM.
use anyhow::Result;
use esp_idf_svc::{
    http::{server::EspHttpServer, Method},
    io::Write,
};

pub fn register(server: &mut EspHttpServer<'static>) -> Result<()> {
    const INDEX: &[u8] = include_bytes!("../../web/dist/index.html.gz");
    const JS: &[u8] = include_bytes!("../../web/dist/app.js.gz");
    const CSS: &[u8] = include_bytes!("../../web/dist/app.css.gz");
    const ICON: &[u8] = include_bytes!("../../web/dist/favicon.svg.gz");
    for (path, mime, bytes) in [
        ("/", "text/html; charset=utf-8", INDEX),
        ("/settings", "text/html; charset=utf-8", INDEX),
        ("/home", "text/html; charset=utf-8", INDEX),
        ("/bluetooth", "text/html; charset=utf-8", INDEX),
        ("/system.html", "text/html; charset=utf-8", INDEX),
        ("/logs.html", "text/html; charset=utf-8", INDEX),
        ("/app.js", "text/javascript; charset=utf-8", JS),
        ("/app.css", "text/css; charset=utf-8", CSS),
        ("/favicon.svg", "image/svg+xml", ICON),
    ] {
        server.fn_handler(path, Method::Get, move |req| -> Result<()> {
            let length = bytes.len().to_string();
            let mut response = req.into_response(
                200,
                Some("OK"),
                &[
                    ("Content-Type", mime),
                    ("Content-Encoding", "gzip"),
                    ("Content-Length", &length),
                    ("Cache-Control", "no-cache"),
                    ("X-Content-Type-Options", "nosniff"),
                ],
            )?;
            for chunk in bytes.chunks(4096) {
                response.write_all(chunk)?;
            }
            Ok(())
        })?;
    }
    Ok(())
}
