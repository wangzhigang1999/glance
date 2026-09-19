fn main() {
    let public_build = std::env::var("RLCD_PUBLIC_BUILD").as_deref() == Ok("1");
    println!("cargo:rerun-if-env-changed=RLCD_PUBLIC_BUILD");
    let revision = std::process::Command::new("git")
        .args(["describe", "--always", "--dirty"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=FIRMWARE_REVISION={revision}");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
    let credentials =
        std::env::var("RLCD_IOT_CONFIG").unwrap_or_else(|_| "data/ecs-iot/rlcd-01.json".into());
    println!("cargo:rerun-if-env-changed=RLCD_IOT_CONFIG");
    println!("cargo:rerun-if-changed={credentials}");
    let contents = if public_build {
        b"{}".to_vec()
    } else {
        std::fs::read(&credentials).unwrap_or_else(|_| b"{}".to_vec())
    };
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(output.join("iot_config.json"), contents).unwrap();
    let keys = std::env::var("RLCD_CLOCK_KEYS").unwrap_or_else(|_| "data/bindkeys.json".into());
    println!("cargo:rerun-if-env-changed=RLCD_CLOCK_KEYS");
    println!("cargo:rerun-if-changed={keys}");
    std::fs::write(
        output.join("clock_keys.json"),
        if public_build {
            b"{}".to_vec()
        } else {
            std::fs::read(keys).unwrap_or_else(|_| b"{}".to_vec())
        },
    )
    .unwrap();
    embuild::espidf::sysenv::output();
    println!("cargo:rerun-if-changed=web/prov_form.html");
    println!("cargo:rerun-if-changed=web/prov_done.html");
    println!("cargo:rerun-if-changed=web/live.html");
    println!("cargo:rerun-if-changed=web/settings.html");
    println!("cargo:rerun-if-changed=web/system.html");
}
