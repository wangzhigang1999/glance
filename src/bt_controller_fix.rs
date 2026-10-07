//! esp-idf-svc 0.52.1 uses the nonexistent cfg `esp_idf_version_least_5_5_0`
//! for S3 controller fields. IDF 5.5.3 emits `esp_idf_version_at_least_5_5_0`.
//! Preserve the driver configuration but restore the two runtime capability flags
//! from the same SDK configuration. Remove with the upstream cfg fix.
//! Related upstream report: https://github.com/esp-rs/esp-idf-svc/issues/663
use esp_idf_svc::sys;

extern "C" {
    fn __real_esp_bt_controller_init(
        config: *mut sys::esp_bt_controller_config_t,
    ) -> sys::esp_err_t;
}

#[no_mangle]
unsafe extern "C" fn __wrap_esp_bt_controller_init(
    config: *mut sys::esp_bt_controller_config_t,
) -> sys::esp_err_t {
    let Some(config) = config.as_ref() else {
        return sys::ESP_ERR_INVALID_ARG;
    };
    let mut fixed = *config;
    log::info!(
        "BLE controller compatibility: connect {} -> {}, advertise {} -> {}",
        fixed.connect_en,
        sys::BT_CTRL_BLE_MASTER != 0,
        fixed.adv_en,
        sys::BT_CTRL_BLE_ADV != 0
    );
    fixed.connect_en = sys::BT_CTRL_BLE_MASTER != 0;
    fixed.adv_en = sys::BT_CTRL_BLE_ADV != 0;
    __real_esp_bt_controller_init(&mut fixed)
}
