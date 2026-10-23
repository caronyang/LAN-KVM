fn main() {
    println!("cargo:rerun-if-changed=../../target/release/lankvm-app.exe");
    println!("cargo:rerun-if-changed=../../target/release/lankvm-service.exe");

    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("../../assets/branding/icons/lankvm.ico");
        res.set("ProductName", "LAN-KVM Setup");
        res.set("FileDescription", "LAN-KVM Setup");
        res.set("CompanyName", "LAN-KVM");
        res.set("FileVersion", "0.2.0.0");
        res.set("ProductVersion", "0.2.0.0");
        if let Err(err) = res.compile() {
            println!("cargo:warning=setup icon resource failed: {err}");
        }
    }
}
