fn main() {
    // Version comes from Cargo so the exe metadata cannot drift from the crate
    // version again (it was hard-coded to 1.1.6 while the crate moved on).
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");

    #[cfg(windows)]
    {
        let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
        let numeric = format!("{version}.0");

        let mut res = winres::WindowsResource::new();
        res.set_icon("../../assets/branding/icons/lankvm.ico");
        res.set("ProductName", "LAN-KVM");
        res.set("FileDescription", "LAN-KVM");
        res.set("CompanyName", "LAN-KVM");
        res.set("FileVersion", &numeric);
        res.set("ProductVersion", &numeric);
        if let Err(err) = res.compile() {
            println!("cargo:warning=icon resource failed: {err}");
        }
    }
}
