//! Embeds the app icon in the Windows executable and passes build information to
//! Help > About.

fn main() {
    for (name, var) in [
        ("MT_BUILD_TARGET", "TARGET"),
        ("MT_BUILD_PROFILE", "PROFILE"),
    ] {
        let value = std::env::var(var).unwrap_or_else(|_| "unknown".into());
        println!("cargo:rustc-env={name}={value}");
    }
    println!("cargo:rerun-if-changed=../../packaging/windows/meshtard.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/windows/meshtard.ico");
        res.compile().expect("embed the Windows icon");
    }
}
