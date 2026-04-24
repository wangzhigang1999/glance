fn main() {
    embuild::espidf::sysenv::output();
    println!("cargo:rerun-if-changed=web/prov_form.html");
    println!("cargo:rerun-if-changed=web/prov_done.html");
    println!("cargo:rerun-if-changed=web/live.html");
    println!("cargo:rerun-if-changed=web/settings.html");
}
