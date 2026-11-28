fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");

    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "EasyTier");
        res.set("FileDescription", "EasyTier Native Client");
        res.set("OriginalFilename", "easytier-native-ui.exe");
        res.compile().expect("compile Windows icon resources");
    }
}