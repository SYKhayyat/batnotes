use anyhow::Result;
use batnotes::app::App;
use batnotes::hotkey;
use batnotes::platform;
use eframe::egui;

fn main() -> Result<()> {
    // Fix the loader path before anything touches a windowing library. This may
    // re-exec the process.
    platform::prepare();

    // Single instance. A second `batnotes --show` — which is what the desktop
    // shortcut launches — hands the request to the running daemon and exits, so
    // the shortcut never opens a competing window.
    let listener = hotkey::serve_single_instance()?;
    if listener.is_none() {
        return Ok(());
    }
    if let Some(l) = listener {
        hotkey::publish_listener(l);
    }

    let viewport = egui::ViewportBuilder::default()
        .with_title("batnotes")
        .with_inner_size([600.0, 460.0])
        .with_min_inner_size([400.0, 240.0])
        .with_decorations(false)
        .with_resizable(true)
        .with_always_on_top()
        .with_visible(false)
        .with_app_id("batnotes");

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    match eframe::run_native(
        "batnotes",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)?))),
    ) {
        Ok(()) => Ok(()),
        // winit can only build one event loop per process, so the XWayland
        // fallback has to be a fresh process rather than a second call.
        Err(e) => platform::retry_on_xwayland(&format!("batnotes failed to start: {e}")),
    }
}