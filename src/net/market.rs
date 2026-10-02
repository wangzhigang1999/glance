//! Compact market snapshot fetched from the companion quant project.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use esp_idf_svc::http::{
    client::{Configuration, EspHttpConnection},
    Method,
};

const MAX_BODY: usize = 8 * 1024;
const MAX_TREND: usize = 18;
const MAX_WATCH: usize = 6;

#[derive(Clone, Debug, Default)]
pub struct MarketIndex {
    pub label: String,
    pub price: f32,
    pub pct: f32,
    pub trend: Vec<f32>,
}

#[derive(Clone, Debug, Default)]
pub struct MarketBreadth {
    pub rise: u32,
    pub flat: u32,
    pub fall: u32,
    pub rise_pct: f32,
    pub limit_up: u32,
    pub limit_down: u32,
}

#[derive(Clone, Debug, Default)]
pub struct MarketWatch {
    pub code: String,
    pub price: f32,
    pub pct: f32,
    pub candles: Vec<MarketCandle>,
}

#[derive(Clone, Debug, Default)]
pub struct MarketCandle {
    pub open: f32,
    pub high: f32,
    pub low: f32,
    pub close: f32,
}

#[derive(Clone, Debug, Default)]
pub struct MarketAlert {
    pub severity: String,
    pub label: String,
    pub condition_count: u32,
}

#[derive(Clone, Debug, Default)]
pub struct MarketData {
    pub generated_at: u64,
    pub updated_hm: String,
    pub session: String,
    pub candle_asof: String,
    pub indices: Vec<MarketIndex>,
    pub breadth: MarketBreadth,
    pub watch: Vec<MarketWatch>,
    pub alert: MarketAlert,
}

#[derive(serde::Deserialize)]
struct RawFeed {
    version: u32,
    generated_at: u64,
    updated_hm: String,
    session: String,
    #[serde(default)]
    candle_asof: Option<String>,
    #[serde(default)]
    indices: Vec<RawIndex>,
    #[serde(default)]
    breadth: RawBreadth,
    #[serde(default)]
    watch: Vec<RawWatch>,
    #[serde(default)]
    alert: RawAlert,
}

#[derive(serde::Deserialize)]
struct RawIndex {
    label: String,
    price: f32,
    pct: f32,
    #[serde(default)]
    trend: Vec<f32>,
}

#[derive(serde::Deserialize, Default)]
struct RawBreadth {
    rise: u32,
    flat: u32,
    fall: u32,
    rise_pct: f32,
    limit_up: u32,
    limit_down: u32,
}

#[derive(serde::Deserialize)]
struct RawWatch {
    code: String,
    price: f32,
    pct: f32,
    #[serde(default)]
    candles: Vec<[f32; 4]>,
}

#[derive(serde::Deserialize, Default)]
struct RawAlert {
    severity: String,
    label: String,
    condition_count: u32,
}

pub fn fetch(url: &str, token: Option<&str>) -> Result<MarketData> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(anyhow!("market URL must start with http:// or https://"));
    }
    let cfg = Configuration {
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        timeout: Some(Duration::from_secs(15)),
        buffer_size: Some(4096),
        buffer_size_tx: Some(1024),
        ..Default::default()
    };
    let mut conn = EspHttpConnection::new(&cfg)?;
    let auth = token.map(|value| format!("Bearer {value}"));
    let mut headers: heapless::Vec<(&str, &str), 3> = heapless::Vec::new();
    let _ = headers.push(("user-agent", "glance/0.1"));
    let _ = headers.push(("accept", "application/json"));
    if let Some(value) = auth.as_deref() {
        let _ = headers.push(("authorization", value));
    }
    conn.initiate_request(Method::Get, url, &headers)?;
    conn.initiate_response()?;
    let status = conn.status();
    if status != 200 {
        return Err(anyhow!("market HTTP {status}"));
    }

    let mut body = Vec::with_capacity(2048);
    let mut chunk = [0u8; 512];
    loop {
        let n = conn
            .read(&mut chunk)
            .map_err(|e| anyhow!("market read: {e:?}"))?;
        if n == 0 {
            break;
        }
        if body.len() + n > MAX_BODY {
            return Err(anyhow!("market payload exceeds {MAX_BODY} bytes"));
        }
        body.extend_from_slice(&chunk[..n]);
    }

    let raw: RawFeed = serde_json::from_slice(&body).context("market JSON")?;
    if raw.version != 1 {
        return Err(anyhow!("unsupported market feed version {}", raw.version));
    }

    Ok(MarketData {
        generated_at: raw.generated_at,
        updated_hm: truncate(raw.updated_hm, 5),
        session: truncate(raw.session, 8),
        candle_asof: truncate(raw.candle_asof.unwrap_or_default(), 10),
        indices: raw
            .indices
            .into_iter()
            .take(2)
            .map(|index| MarketIndex {
                label: truncate(index.label, 10),
                price: index.price,
                pct: index.pct,
                trend: index.trend.into_iter().take(MAX_TREND).collect(),
            })
            .collect(),
        breadth: MarketBreadth {
            rise: raw.breadth.rise,
            flat: raw.breadth.flat,
            fall: raw.breadth.fall,
            rise_pct: raw.breadth.rise_pct,
            limit_up: raw.breadth.limit_up,
            limit_down: raw.breadth.limit_down,
        },
        watch: raw
            .watch
            .into_iter()
            .take(MAX_WATCH)
            .map(|item| MarketWatch {
                code: truncate(item.code, 8),
                price: item.price,
                pct: item.pct,
                candles: item
                    .candles
                    .into_iter()
                    .take(20)
                    .map(|values| MarketCandle {
                        open: values[0],
                        high: values[1],
                        low: values[2],
                        close: values[3],
                    })
                    .collect(),
            })
            .collect(),
        alert: MarketAlert {
            severity: truncate(raw.alert.severity, 8),
            label: truncate(raw.alert.label, 31),
            condition_count: raw.alert.condition_count,
        },
    })
}

fn truncate(mut value: String, max_chars: usize) -> String {
    if value.chars().count() > max_chars {
        value = value.chars().take(max_chars).collect();
    }
    value
}
