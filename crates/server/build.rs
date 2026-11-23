// Windows only: put the icon and the version information into family-app.exe (Explorer and the taskbar show them).
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Family Tree");
        res.set("FileDescription", "Family Tree");
        res.set("OriginalFilename", "family-app.exe");
        if let Err(e) = res.compile() {
            // A missing resource compiler must not stop the build; the exe just has no icon.
            println!("cargo:warning=could not embed the icon: {e}");
        }
    }
}
