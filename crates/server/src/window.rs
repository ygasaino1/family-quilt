//! The app window: the OS web view (WebView2 on Windows, WKWebView on macOS, WebKitGTK on Linux) showing the UI that
//! this process serves on 127.0.0.1. Built only with `--features window`.
//!
//! Not verified on this project's Linux build machine (no WebKitGTK there); the Windows build is checked by the
//! `windows` GitHub Actions job.

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

/// Plain-language text for the case where the web view is missing (WebView2 on older Windows 10).
pub const MISSING_VIEW: &str = "The Family Tree window needs the Microsoft Edge WebView2 Runtime, which this computer does not seem to have.\n\nYou can get it free from:\nhttps://developer.microsoft.com/microsoft-edge/webview2/\n\nOr start this program with --browser to use your web browser instead.";

/// Show a message the person can read even though the program has no console.
pub fn message(title: &str, text: &str) {
    rfd::MessageDialog::new().set_title(title).set_description(text).set_level(rfd::MessageLevel::Error).show();
}

/// Open the window and run until it is closed. Returns an error text if the web view cannot start.
pub fn run(url: &str, title: &str) -> Result<(), String> {
    wry::webview_version().map_err(|_| MISSING_VIEW.to_string())?;
    let mut event_loop = EventLoop::new();
    let window =
        WindowBuilder::new().with_title(title).with_inner_size(LogicalSize::new(1280.0, 820.0)).with_min_inner_size(LogicalSize::new(760.0, 520.0)).build(&event_loop).map_err(|e| e.to_string())?;
    let webview = WebViewBuilder::new().with_url(url).build(&window).map_err(|e| format!("{MISSING_VIEW}\n\n({e})"))?;
    event_loop.run_return(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        let _keep_alive = &webview;
        if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
            *flow = ControlFlow::Exit;
        }
    });
    Ok(())
}

/// The system "choose a folder" dialog. `None` when the person cancels.
pub fn pick_folder() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new().set_title("Where should the family tree be kept?").pick_folder()
}
