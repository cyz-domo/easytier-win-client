fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-env-changed=EASYTIER_APP_VERSION");

    let version = std::env::var("EASYTIER_APP_VERSION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=EASYTIER_APP_VERSION={version}");

    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "EasyTier");
        res.set("FileDescription", "EasyTier Native Client");
        res.set("OriginalFilename", "easytier-native-ui.exe");
        res.set("FileVersion", &version);
        res.set("ProductVersion", &version);
        res.compile().expect("compile Windows icon resources");
    }
}