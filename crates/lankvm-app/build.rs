fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("../../assets/branding/icons/lankvm.ico");
        res.set("ProductName", "LAN-KVM");
        res.set("FileDescription", "LAN-KVM");
        res.set("CompanyName", "LAN-KVM");
        res.set("FileVersion", "1.0.0.0");
        res.set("ProductVersion", "1.0.0.0");
        if let Err(err) = res.compile() {
            println!("cargo:warning=icon resource failed: {err}");
        }
    }
}
