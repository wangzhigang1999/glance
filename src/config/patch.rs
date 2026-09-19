use anyhow::Result;
/// 入站 patch:所有字段 Option,缺省表示不更新。脱敏 token(`***...`)视为不更新。
#[derive(serde::Deserialize, Default)]
struct ConfigPatch {
    gh_user: Option<String>,
    gh_token: Option<String>,
    gh_refresh_s: Option<u32>,
    gh_err_s: Option<u32>,
    market_url: Option<String>,
    market_token: Option<String>,
    market_refresh_s: Option<u32>,
    sensor_refresh_s: Option<u32>,
    auto_rotate: Option<bool>,
    auto_rotate_s: Option<u32>,
    temp_off_c: Option<f32>,
    humid_off_pct: Option<f32>,
    tz_off_s: Option<i32>,
    splash_flash: Option<u32>,
}

pub fn apply_json_patch(c: &mut crate::config::RuntimeConfig, body: &str) -> Result<()> {
    let p: ConfigPatch = serde_json::from_str(body)?;
    if let Some(user) = &p.gh_user {
        anyhow::ensure!(
            user.len() <= 39 && user.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
            "Invalid GitHub username"
        );
    }
    if let Some(v) = p.gh_user {
        c.gh_user = v;
    }
    if let Some(v) = p.gh_token {
        if v.is_empty() {
            c.gh_token.clear();
        } else if !v.starts_with("***") {
            c.gh_token = v;
        }
    }
    if let Some(v) = p.gh_refresh_s {
        c.gh_refresh_s = v;
    }
    if let Some(v) = p.gh_err_s {
        c.gh_err_s = v;
    }
    if let Some(v) = p.market_url {
        c.market_url = v;
    }
    if let Some(v) = p.market_token {
        if v.is_empty() {
            c.market_token.clear();
        } else if !v.starts_with("***") {
            c.market_token = v;
        }
    }
    if let Some(v) = p.market_refresh_s {
        c.market_refresh_s = v;
    }
    if let Some(v) = p.sensor_refresh_s {
        c.sensor_refresh_s = v;
    }
    if let Some(v) = p.auto_rotate {
        c.auto_rotate = v;
    }
    if let Some(v) = p.auto_rotate_s {
        c.auto_rotate_s = v;
    }
    if let Some(v) = p.temp_off_c {
        c.temp_off_c = v;
    }
    if let Some(v) = p.humid_off_pct {
        c.humid_off_pct = v;
    }
    if let Some(v) = p.tz_off_s {
        c.tz_off_s = v;
    }
    if let Some(v) = p.splash_flash {
        c.splash_flash = v;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_request_does_not_mutate_configuration() {
        let mut c = crate::config::RuntimeConfig::default();
        c.gh_refresh_s = 300;
        for body in [
            "{broken",
            r#"{"gh_user":"中文","gh_refresh_s":30}"#,
            r#"{"gh_refresh_s":"invalid"}"#,
        ] {
            assert!(apply_json_patch(&mut c, body).is_err());
            assert_eq!(c.gh_refresh_s, 300);
        }
    }
    #[test]
    fn masked_token_and_partial_patch() {
        let mut c = crate::config::RuntimeConfig::default();
        c.gh_token = "synthetic-test".into();
        apply_json_patch(&mut c, r#"{"gh_token":"***test","gh_refresh_s":600}"#).unwrap();
        assert_eq!(c.gh_token, "synthetic-test");
        assert_eq!(c.gh_refresh_s, 600);
        apply_json_patch(&mut c, r#"{"gh_token":""}"#).unwrap();
        assert!(c.gh_token.is_empty());
    }
    #[test]
    fn actual_config_clamp_handles_multibyte_boundary() {
        let mut c = crate::config::RuntimeConfig::default();
        c.gh_user = format!("{}你", "a".repeat(38));
        crate::config::clamp(&mut c);
        assert_eq!(c.gh_user.len(), 38);
    }
}
