use std::time::Duration;

use anyhow::Result;
use batnotes::app::App;
use batnotes::hotkey;
use batnotes::platform;
use batnotes::ui::gtk;
use gtk4::glib;

// Keep the UI alive for the life of the process. The overlay outlives having
// no visible window, so the window is created once and only ever hidden. A
// GTK `Application` would tie the process lifetime to window destruction,
// which is the opposite of what a daemon needs.
thread_local! {
    static UI: std::cell::RefCell<Option<std::rc::Rc<gtk::Ui>>> =
        const { std::cell::RefCell::new(None) };
}

fn main() -> Result<()> {
    platform::install_panic_log();
    platform::prepare();

    // Single instance. A second `batnotes --show` — which is what the desktop
    // shortcut launches — hands the request to the running daemon and exits.
    let listener = hotkey::serve_single_instance()?;
    if listener.is_none() {
        return Ok(());
    }
    if let Some(l) = listener {
        hotkey::publish_listener(l);
    }

    // GTK initialises the display backend; everything that touches widgets has
    // to come after this.
    gtk4::init().map_err(|_| anyhow::anyhow!("gtk::init failed — no display available"))?;

    let state = App::new()?;
    let window = gtk4::Window::builder()
        .title("batnotes")
        .default_width(620)
        .default_height(470)
        .resizable(true)
        .build();

    let ui = gtk::build(state, window);
    UI.with(|slot| *slot.borrow_mut() = Some(ui.clone()));


    // Hotkey events, search results, toast expiry and focus loss all arrive
    // through this tick, so the daemon keeps working while hidden.
    glib::timeout_add_local(Duration::from_millis(50), move || {
        ui.clone().tick();
        glib::ControlFlow::Continue
    });
    if std::env::var_os("BATNOTES_DEBUG_UI").is_some() {
        eprintln!("batnotes: glib main loop running, tick scheduled");
    }

    // Pump the *default* main context, because that is where
    // `timeout_add_local` and GTK's own sources are registered. Running a
    // separate `MainLoop` leaves the default context untouched, so no timeout
    // ever fires and the overlay is never shown.
    let ctx = glib::MainContext::default();
    loop {
        ctx.iteration(false);
        std::thread::sleep(Duration::from_millis(8));
    }
}