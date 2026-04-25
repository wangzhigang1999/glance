/* esp-sr AFE FFI binding surface.
 *
 * esp-idf-sys 的 bindgen 读这个头,把里面 include 的 esp-sr 公共 API 生成成
 * `esp_idf_svc::sys::afe::*`(由 Cargo.toml 的 bindings_module = "afe" 决定)。
 *
 * 只 include 我们真正用到的 C 接口,把内部 C++ 实现头排除在外,避免 bindgen 在
 * 模板/STL 上炸。
 */

#pragma once

#if defined(ESP_IDF_COMP_ESPRESSIF__ESP_SR_ENABLED)
#include "esp_afe_config.h"     /* afe_config_t / afe_config_init / afe_type_t / vad_mode_t */
#include "esp_afe_sr_iface.h"   /* esp_afe_sr_iface_t / afe_fetch_result_t / afe_vad_state_t */
#include "esp_afe_sr_models.h"  /* esp_afe_handle_from_config */
#include "model_path.h"         /* esp_srmodel_init / srmodel_list_t */
#endif
