fn main() {
    let credentials =
        std::env::var("RLCD_IOT_CONFIG").unwrap_or_else(|_| "data/ecs-iot/rlcd-01.json".into());
    println!("cargo:rerun-if-env-changed=RLCD_IOT_CONFIG");
    println!("cargo:rerun-if-changed={credentials}");
    let contents = std::fs::read(&credentials).unwrap_or_else(|_| b"{}".to_vec());
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(output.join("iot_config.json"), contents).unwrap();
    let keys = std::env::var("RLCD_CLOCK_KEYS").unwrap_or_else(|_| "data/bindkeys.json".into());
    println!("cargo:rerun-if-env-changed=RLCD_CLOCK_KEYS");
    println!("cargo:rerun-if-changed={keys}");
    std::fs::write(
        output.join("clock_keys.json"),
        std::fs::read(keys).unwrap_or_else(|_| b"{}".to_vec()),
    )
    .unwrap();
    embuild::espidf::sysenv::output();
    println!("cargo:rerun-if-changed=web/prov_form.html");
    println!("cargo:rerun-if-changed=web/prov_done.html");
    println!("cargo:rerun-if-changed=web/live.html");
    println!("cargo:rerun-if-changed=web/settings.html");
    println!("cargo:rerun-if-changed=web/system.html");
}
