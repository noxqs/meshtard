//! Embeds the app icon in the Windows executable.

fn main() {
    println!("cargo:rerun-if-changed=../../packaging/windows/meshtard.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/windows/meshtard.ico");
        res.compile().expect("embed the Windows icon");
    }
}
