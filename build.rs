fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let mut res = winres::WindowsResource::new();
        res.set_icon("c.ico");
        res.compile().expect("embed icon");
    }
    println!("cargo:rerun-if-changed=c.ico");
    println!("cargo:rerun-if-changed=build.rs");
}
