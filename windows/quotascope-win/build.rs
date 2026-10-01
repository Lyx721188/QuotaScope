fn main() {
    // Loose WinUI resources and the DirectWrite collection share these
    // unmodified font files beside the executable, including the license.
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let target = out.ancestors().nth(3).unwrap().join("Fonts");
    std::fs::create_dir_all(&target).unwrap();
    println!("cargo:rerun-if-changed=assets/fonts");
    for entry in std::fs::read_dir("assets/fonts").unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
    }
    // Ship the Windows App SDK runtime next to quotascope.exe, so the WinUI 3
    // settings window works without a framework-package install.
    windows_reactor_setup::as_self_contained();

    // The bundled Windows icon plus the version strings Explorer and the
    // taskbar read.
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/app.ico");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    res.set("FileDescription", "QuotaScope");
    res.set("ProductName", "QuotaScope");
    res.set("FileVersion", &version);
    res.set("ProductVersion", &version);
    if let Err(e) = res.compile() {
        // A missing icon resource should never fail the build; the exe
        // just falls back to the generic one.
        eprintln!("winresource: {e}");
    }
}
