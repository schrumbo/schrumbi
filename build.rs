fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("packaging/icons/schrumbi.ico");
        res.set("ProductName", "Schrumbi");
        res.set("FileDescription", "Schrumbi");
        let _ = res.compile();
    }
}
