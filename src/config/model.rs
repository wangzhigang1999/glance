#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    // --- GitHub 身份 ---
    pub gh_user: String,
    pub gh_token: String,
    // --- GitHub fetcher 周期 (秒);contrib/notif/activity 共用同一 worker 串行轮询 ---
    pub gh_refresh_s: u32,
    pub gh_err_s: u32,
    // --- Market feed (quant 项目局域网/ECS 只读 JSON) ---
    pub market_url: String,
    pub market_token: String,
    pub market_refresh_s: u32,
    // --- 主循环 ---
    pub sensor_refresh_s: u32,
    pub auto_rotate: bool,
    pub auto_rotate_s: u32,
    // --- 传感器校准 ---
    pub temp_off_c: f32,
    pub humid_off_pct: f32,
    // --- 时区 ---
    pub tz_off_s: i32,
    // --- 开机屏幕除斑次数 (仅启动时读) ---
    pub splash_flash: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            gh_user: String::new(),
            gh_token: String::new(),
            gh_refresh_s: 300,
            gh_err_s: 120,
            market_url: String::new(),
            market_token: String::new(),
            market_refresh_s: 60,
            sensor_refresh_s: 5,
            auto_rotate: false,
            auto_rotate_s: 15,
            temp_off_c: 0.0,
            humid_off_pct: 0.0,
            tz_off_s: 8 * 3600,
            splash_flash: 3,
        }
    }
}

/// 合法化:夹取到合理区间,防止意外值把设备搞成不可用
pub fn clamp(c: &mut RuntimeConfig) {
    fn clip(v: &mut u32, lo: u32, hi: u32) {
        if *v < lo {
            *v = lo;
        } else if *v > hi {
            *v = hi;
        }
    }
    clip(&mut c.gh_refresh_s, 30, 24 * 3600);
    clip(&mut c.gh_err_s, 30, 24 * 3600);
    clip(&mut c.market_refresh_s, 15, 24 * 3600);
    clip(&mut c.sensor_refresh_s, 1, 3600);
    clip(&mut c.auto_rotate_s, 3, 3600);
    clip(&mut c.splash_flash, 0, 64);
    // 温/湿度偏移
    if c.temp_off_c < -20.0 {
        c.temp_off_c = -20.0;
    }
    if c.temp_off_c > 20.0 {
        c.temp_off_c = 20.0;
    }
    if c.humid_off_pct < -50.0 {
        c.humid_off_pct = -50.0;
    }
    if c.humid_off_pct > 50.0 {
        c.humid_off_pct = 50.0;
    }
    if c.tz_off_s < -14 * 3600 {
        c.tz_off_s = -14 * 3600;
    }
    if c.tz_off_s > 14 * 3600 {
        c.tz_off_s = 14 * 3600;
    }
    // 字符串长度保护
    if c.gh_user.len() > 39 {
        crate::reliability::truncate_utf8(&mut c.gh_user, 39);
    }
    if c.gh_token.len() > 255 {
        crate::reliability::truncate_utf8(&mut c.gh_token, 255);
    }
    if c.market_url.len() > 191 {
        crate::reliability::truncate_utf8(&mut c.market_url, 191);
    }
    if c.market_token.len() > 127 {
        crate::reliability::truncate_utf8(&mut c.market_token, 127);
    }
}
