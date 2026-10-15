//! Global hotkeys.
//!
//! Three backends, tried in order of how well they respect the session type:
//!
//! 1. [`Backend::Portal`] — `org.freedesktop.portal.GlobalShortcuts`. The only
//!    approach that works for Wayland-native clients, because the compositor
//!    itself owns the binding.
//! 2. [`Backend::X11`] — `XGrabKey` via the `global-hotkey` crate. Only
//!    correct under a real X server, and it is *probed* rather than trusted:
//!    that crate reports `Ok(())` even when its event thread has already died.
//! 3. [`Backend::DesktopShortcut`] — asks the desktop's own shortcut system to
//!    run `batnotes --show`. Not compositor-specific, and it keeps working when
//!    the other two cannot.
//!
//! Every backend can fail, so the service reports which one is live and the UI
//! shows it. It never claims a binding it does not actually hold.

use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub super_: bool,
}

impl Mods {
    pub fn none() -> Mods {
        Mods { ctrl: false, alt: false, shift: false, super_: false }
    }

    fn x11(self) -> global_hotkey::hotkey::Modifiers {
        let mut m = global_hotkey::hotkey::Modifiers::empty();
        if self.ctrl {
            m |= global_hotkey::hotkey::Modifiers::CONTROL;
        }
        if self.alt {
            m |= global_hotkey::hotkey::Modifiers::ALT;
        }
        if self.shift {
            m |= global_hotkey::hotkey::Modifiers::SHIFT;
        }
        if self.super_ {
            m |= global_hotkey::hotkey::Modifiers::META;
        }
        m
    }
}

/// A parsed key combination, e.g. `Meta+Shift+Space`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeySpec {
    pub mods: Mods,
    pub key: String,
}

impl HotkeySpec {
    pub fn parse(s: &str) -> Result<HotkeySpec> {
        let parts: Vec<&str> = s.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            return Err(anyhow!("empty binding"));
        }
        let mut mods = Mods::none();
        let mut key = None;
        for p in &parts {
            match p.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => mods.ctrl = true,
                "alt" | "option" => mods.alt = true,
                "shift" => mods.shift = true,
                "super" | "meta" | "cmd" | "win" => mods.super_ = true,
                other => {
                    if key.is_some() {
                        return Err(anyhow!("two keys in one binding: {s:?}"));
                    }
                    key = Some(lookup_key(other).ok_or_else(|| anyhow!("unknown key {other:?}"))?);
                }
            }
        }
        let key = key.ok_or_else(|| anyhow!("binding has modifiers but no key: {s:?}"))?;
        Ok(HotkeySpec { mods, key })
    }

    /// Canonical form used for conflict detection. Case- and order-insensitive
    /// over modifiers so `shift+ctrl+x` and `Ctrl+Shift+X` collide correctly.
    pub fn normalised(&self) -> String {
        let mut m = String::new();
        if self.mods.ctrl {
            m.push_str("ctrl+");
        }
        if self.mods.alt {
            m.push_str("alt+");
        }
        if self.mods.shift {
            m.push_str("shift+");
        }
        if self.mods.super_ {
            m.push_str("super+");
        }
        format!("{m}{}", self.key.to_ascii_lowercase())
    }

    /// XDG shortcut-trigger syntax, as used by `preferred_trigger`.
    pub fn to_portal_trigger(&self) -> String {
        let mut s = String::new();
        if self.mods.ctrl {
            s.push_str("<Control>");
        }
        if self.mods.alt {
            s.push_str("<Alt>");
        }
        if self.mods.super_ {
            s.push_str("<Super>");
        }
        if self.mods.shift {
            s.push_str("<Shift>");
        }
        s.push_str(&self.key);
        s
    }

    /// The accelerator string KDE's KGlobalAccel writes into
    /// `kglobalshortcutsrc`.
    pub fn to_kde_accelerator(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if self.mods.ctrl {
            parts.push("Ctrl");
        }
        if self.mods.alt {
            parts.push("Alt");
        }
        if self.mods.super_ {
            parts.push("Meta");
        }
        if self.mods.shift {
            parts.push("Shift");
        }
        parts.push(match self.key.as_str() {
            "Space" => "Space",
            "Comma" => "Comma",
            "Slash" => "Slash",
            "BracketLeft" => "BracketLeft",
            "BracketRight" => "BracketRight",
            "Escape" => "Esc",
            "Enter" => "Enter",
            "Up" => "Up",
            "Down" => "Down",
            "Left" => "Left",
            "Right" => "Right",
            "Delete" => "Delete",
            "Backspace" => "Backspace",
            other => other,
        });
        parts.join("+")
    }
}

/// Canonical key names. Everything the UI can display and every backend can
/// consume is in this one table.
fn lookup_key(s: &str) -> Option<String> {
    let lower = s.to_ascii_lowercase();
    // Only a single character is a letter key. Anything longer is either a
    // named key below or a typo, and accepting it silently truncated
    // "NotAKey" to "N".
    if lower.chars().count() == 1 {
        let c = lower.chars().next().unwrap();
        if c.is_ascii_alphanumeric() {
            return Some(c.to_ascii_uppercase().to_string());
        }
    }
    let name = match lower.as_str() {
        "space" | "spacebar" => "Space",
        "enter" | "return" => "Enter",
        "escape" | "esc" => "Escape",
        "tab" => "Tab",
        "backspace" => "Backspace",
        "delete" | "del" => "Delete",
        "up" | "arrowup" => "Up",
        "down" | "arrowdown" => "Down",
        "left" | "arrowleft" => "Left",
        "right" | "arrowright" => "Right",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "comma" => "Comma",
        "period" | "dot" => "Period",
        "slash" => "Slash",
        "backslash" => "Backslash",
        "semicolon" => "Semicolon",
        "apostrophe" | "quote" => "Apostrophe",
        "bracketleft" => "BracketLeft",
        "bracketright" => "BracketRight",
        "minus" | "hyphen" => "Minus",
        "equal" | "equals" => "Equal",
        "grave" | "backtick" => "Grave",
        _ => return None,
    };
    Some(name.to_string())
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Portal,
    X11,
    DesktopShortcut,
    None,
}

impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Backend::Portal => "XDG portal",
            Backend::X11 => "X11 XGrabKey",
            Backend::DesktopShortcut => "desktop shortcut",
            Backend::None => "none",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Status {
    pub backend: Backend,
    /// False when the backend is up but the key it holds is not the one asked
    /// for — e.g. the KDE portal accepted the binding but left the trigger
    /// blank. The UI says so rather than pretending.
    pub trigger_confirmed: bool,
    pub detail: String,
}

pub struct Service {
    pub rx: Receiver<()>,
    pub status: Status,
}

struct Probe {
    tx: Sender<()>,
}

impl Probe {
    fn emit(&self) {
        // Blocking is correct here: these threads exist only to raise the
        // event, and dropping one would lose the activation.
        let _ = self.tx.send(());
    }
}

pub fn start(invoke: HotkeySpec, notes: HotkeySpec, use_portal: bool) -> Service {
    let (tx, rx) = mpsc::channel::<()>();

    // The portal is opt-in and sticky-fail. On KDE Plasma 6 a CreateSession
    // call segfaults xdg-desktop-portal, taking the desktop's file chooser,
    // screenshot and global-shortcut plumbing down with it. That is far too
    // expensive to pay on every launch, so it is never probed implicitly.
    if use_portal && !portal_marked_unusable() {
        if let Some(status) = spawn_portal(tx.clone(), &invoke) {
            log(&status, &invoke);
            return Service { rx, status };
        }
        mark_portal_unusable();
    }

    if let Some(status) = spawn_x11(tx.clone(), &invoke) {
        log(&status, &invoke);
        return Service { rx, status };
    }
    if let Some(status) = spawn_x11(tx.clone(), &invoke) {
        log(&status, &invoke);
        return Service { rx, status };
    }
    // Last resort: hand the binding to the desktop's own shortcut system, and
    // then check whether it actually took. Writing kglobalshortcutsrc is not
    // the same as the daemon having read it, and claiming otherwise is the bug
    // that started all of this.
    let status = match install_desktop_shortcut(&invoke, &notes) {
        Ok(()) => {
            let live = kde_component_registered();
            Status {
                backend: Backend::DesktopShortcut,
                trigger_confirmed: live,
                detail: if live {
                    format!("KGlobalAccel is serving {} — press it to run `batnotes --show`", invoke.to_kde_accelerator())
                } else {
                    format!(
                        "{} written to kglobalshortcutsrc, but the shortcut daemon has not reloaded it. Log out and back in (or restart plasma-kglobalaccel) to activate it. Until then, run `batnotes --show`.",
                        invoke.to_kde_accelerator()
                    )
                },
            }
        }
        Err(e) => Status {
            backend: Backend::None,
            trigger_confirmed: false,
            detail: format!("No global hotkey backend available: {e}"),
        },
    };
    log(&status, &invoke);
    Service { rx, status }
}

/// One line on stderr at startup. A hotkey that silently does nothing is the
/// failure mode this whole module exists to prevent, so say what we got.
fn log(status: &Status, invoke: &HotkeySpec) {
    eprintln!(
        "batnotes: hotkey {} via {} ({})",
        invoke.to_kde_accelerator(),
        status.backend.label(),
        if status.trigger_confirmed { "confirmed" } else { "requested" },
    );
}

// ---- portal --------------------------------------------------------------

/// How long to wait for the portal before declaring it unusable. Some
/// implementations open a configuration dialog here and never answer.
const PORTAL_TIMEOUT: Duration = Duration::from_secs(4);

fn portal_state_dir() -> PathBuf {
    dirs::state_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("batnotes")
}

fn portal_marker() -> PathBuf {
    portal_state_dir().join("portal-unusable")
}

/// Has the portal already proven hostile on this machine?
pub fn portal_marked_unusable() -> bool {
    portal_marker().exists()
}

fn mark_portal_unusable() {
    let path = portal_marker();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, "the GlobalShortcuts portal did not answer; not probing again\n");
}

/// Clear the marker so the portal can be retried from Settings.
pub fn forget_portal_failure() {
    let _ = std::fs::remove_file(portal_marker());
}

fn spawn_portal(tx: Sender<()>, invoke: &HotkeySpec) -> Option<Status> {
    let trigger = invoke.to_portal_trigger();

    // The handshake runs here, on the caller's thread, behind a timeout.
    // Doing it in the background and assuming success is what makes an app
    // report a hotkey it does not have: on this machine the KDE portal
    // disconnects during CreateSession, and the thread just dies quietly.
    let (tx_probe, rx) = mpsc::channel::<Result<Portal, String>>();
    let _ = std::thread::Builder::new()
        .name("batnotes-portal".into())
        .spawn(move || {
            let _ = tx_probe.send(handshake(&trigger).map_err(|e| format!("{e}")));
        });

    let portal = match rx.recv_timeout(PORTAL_TIMEOUT) {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            eprintln!("batnotes: portal hotkey unavailable: {e} — not probing it again");
            return None;
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            eprintln!("batnotes: portal did not answer within {PORTAL_TIMEOUT:?}; skipping it");
            return None;
        }
        Err(_) => return None,
    };

    // Read the status before the connection moves into the listener thread.
    let granted = portal.trigger_description.clone();
    let probe = Arc::new(Probe { tx });
    let _ = std::thread::Builder::new()
        .name("batnotes-portal-events".into())
        .spawn(move || {
            portal.run_activations(probe);
            eprintln!("batnotes: portal activation stream ended");
        });

    let confirmed = granted.as_deref().is_some_and(|t| !t.is_empty());
    Some(Status {
        backend: Backend::Portal,
        trigger_confirmed: confirmed,
        detail: match granted.as_deref() {
            Some(t) if !t.is_empty() => format!("bound as {t}"),
            Some(_) => "bound, but the key still needs assigning in system settings".to_string(),
            None => "bound".to_string(),
        },
    })
}

struct Portal {
    conn: zbus::blocking::connection::Connection,
    _session: zvariant::OwnedObjectPath,
    trigger_description: Option<String>,
}

impl Portal {
    /// Block on the Activated signal, firing `probe` once per press.
    fn run_activations(self, probe: Arc<Probe>) {
        use zbus::blocking::Proxy;
        use zvariant::OwnedObjectPath;
        const DEST: &str = "org.freedesktop.portal.Desktop";
        const PATH: &str = "/org/freedesktop/portal/desktop";
        let iface: zbus_names::InterfaceName = "org.freedesktop.portal.GlobalShortcuts"
            .try_into()
            .expect("static interface name");
        let Ok(proxy) = Proxy::new(&self.conn, DEST, PATH, iface) else { return };
        let Ok(mut it) = proxy.receive_signal("Activated") else { return };
        while let Some(msg) = it.next() {
            let body = msg.body();
            let parsed: std::result::Result<
                (OwnedObjectPath, String, u32, std::collections::HashMap<String, zvariant::OwnedValue>),
                _,
            > = body.deserialize();
            if let Ok((_session, id, _t, _opts)) = parsed {
                if id == "batnotes-invoke" {
                    probe.emit();
                }
            }
        }
    }
}


fn handshake(trigger: &str) -> Result<Portal> {
    use std::collections::HashMap;
    use zbus::blocking::{connection::Connection, Proxy};
    use zvariant::{OwnedObjectPath, OwnedValue, Str, Value};

    const DEST: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";

    let conn = Connection::session().map_err(|e| anyhow!("no session bus: {e}"))?;
    let iface: zbus_names::InterfaceName = "org.freedesktop.portal.GlobalShortcuts".try_into().unwrap();
    let portal = Proxy::new(&conn, DEST, PATH, iface).map_err(|e| anyhow!("{e}"))?;

    let token = std::process::id();
    let mut opts = HashMap::new();
    opts.insert("handle_token", Value::from(format!("bn-{token}")));
    opts.insert("session_handle_token", Value::from(format!("bn-{token}")));
    let request: OwnedObjectPath = portal
        .call("CreateSession", &opts)
        .map_err(|e| anyhow!("CreateSession: {e}"))?;

    // CreateSession returns the *request* handle. The session handle arrives on
    // Request::Response; BindShortcuts rejects anything else with
    // "Invalid session".
    let session = await_response(&conn, &request)?
        .get("session_handle")
        .and_then(session_handle_from)
        .ok_or_else(|| anyhow!("CreateSession response carried no session_handle"))?;

    let mut shortcut = HashMap::new();
    shortcut.insert(
        "description".to_string(),
        OwnedValue::from(Str::from("Show batnotes overlay")),
    );
    shortcut.insert(
        "preferred_trigger".to_string(),
        OwnedValue::from(Str::from(trigger.to_string())),
    );
    let mut bopts = HashMap::new();
    bopts.insert("handle_token", Value::from(format!("bnb-{token}")));
    let bind_request: OwnedObjectPath = portal
        .call(
            "BindShortcuts",
            &(session.as_ref(), vec![("batnotes-invoke".to_string(), shortcut)], String::new(), bopts),
        )
        .map_err(|e| anyhow!("BindShortcuts: {e}"))?;

    let results = await_response(&conn, &bind_request)?;
    Ok(Portal {
        conn,
        _session: session,
        trigger_description: trigger_description(&results),
    })
}

fn await_response(
    conn: &zbus::blocking::connection::Connection,
    request: &zvariant::OwnedObjectPath,
) -> Result<std::collections::HashMap<String, zvariant::OwnedValue>> {
    use zbus::blocking::Proxy;
    const DEST: &str = "org.freedesktop.portal.Desktop";
    let iface: zbus_names::InterfaceName = "org.freedesktop.portal.Request".try_into().unwrap();
    let p = Proxy::new(conn, DEST, request.as_ref(), iface).map_err(|e| anyhow!("{e}"))?;
    let mut it = p.receive_signal("Response").map_err(|e| anyhow!("{e}"))?;
    let msg = it.next().ok_or_else(|| anyhow!("no Response signal"))?;
    let body = msg.body();
    let (_code, results): (u32, std::collections::HashMap<String, zvariant::OwnedValue>) =
        body.deserialize()?;
    Ok(results)
}

fn session_handle_from(v: &zvariant::OwnedValue) -> Option<zvariant::OwnedObjectPath> {
    use zvariant::{OwnedValue, Str};
    OwnedValue::try_clone(v)
        .and_then(|o| Str::try_from(o))
        .ok()
        .map(|s| s.to_string())
        .and_then(|s| zvariant::OwnedObjectPath::try_from(s).ok())
}

/// Pull `trigger_description` out of the BindShortcuts response.
fn trigger_description(results: &std::collections::HashMap<String, zvariant::OwnedValue>) -> Option<String> {
    use zvariant::Value;
    let raw = results.get("shortcuts")?;
    let value: Value<'_> = raw.try_into().ok()?;
    let array = match value {
        Value::Array(a) => a,
        _ => return None,
    };
    for entry in array.iter() {
        let fields = match entry {
            Value::Structure(s) => s.fields(),
            _ => continue,
        };
        for field in fields {
            let dict = match field {
                Value::Dict(d) => d,
                _ => continue,
            };
            for (k, val) in dict.iter() {
                let key = match k {
                    Value::Str(s) => s.to_string(),
                    _ => continue,
                };
                if key == "trigger_description" {
                    let text = match val {
                        Value::Str(st) => st.to_string(),
                        _ => continue,
                    };
                    return Some(text);
                }
            }
        }
    }
    None
}

// ---- X11 -----------------------------------------------------------------

fn spawn_x11(tx: Sender<()>, invoke: &HotkeySpec) -> Option<Status> {
    // global-hotkey returns Ok(()) even when its event thread has already
    // exited, so probe the library and the display ourselves first. Without
    // this the app reports a working hotkey that can never fire.
    if !x11_available() {
        return None;
    }
    use global_hotkey::hotkey::HotKey;
    use global_hotkey::GlobalHotKeyEvent;

    let Some(code) = to_x11_code(&invoke.key) else {
        return None;
    };
    let hk = HotKey::new(Some(invoke.mods.x11()), code);
    let id = hk.id();
    let probe = Arc::new(Probe { tx });

    let mgr = match global_hotkey::GlobalHotKeyManager::new() {
        Ok(m) => m,
        Err(_) => return None,
    };
    if mgr.register(hk).is_err() {
        return None;
    }

    let _ = std::thread::Builder::new().name("batnotes-x11".into()).spawn(move || loop {
        if let Ok(ev) = GlobalHotKeyEvent::receiver().recv_timeout(Duration::from_millis(250)) {
            if ev.id == id && ev.state == global_hotkey::HotKeyState::Pressed {
                probe.emit();
            }
        }
    });

    // The manager owns a background thread that must outlive this call. There
    // is no Drop that shuts it down cleanly, so it is leaked deliberately.
    let _leaked: &'static mut global_hotkey::GlobalHotKeyManager = Box::leak(Box::new(mgr));

    Some(Status {
        backend: Backend::X11,
        trigger_confirmed: true,
        detail: "XGrabKey — only fires while an X11 client has focus".into(),
    })
}

/// Is this a real X11 session where an XGrabKey would actually be received?
///
/// `DISPLAY` being set proves nothing: XWayland sets it even on a pure Wayland
/// session, and a grab on the XWayland root window never sees keystrokes typed
/// into a native Wayland client. The session type is the honest test, and
/// without it this backend would claim success while being unable to fire.
fn x11_available() -> bool {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return false;
    }
    match std::env::var("XDG_SESSION_TYPE").as_deref() {
        Ok("x11") | Ok("xorg") => {}
        Ok(_) => return false,
        Err(_) => {}
    }
    if std::env::var_os("DISPLAY").is_none() {
        return false;
    }
    x11_dl::xlib::Xlib::open().is_ok()
}

fn to_x11_code(key: &str) -> Option<global_hotkey::hotkey::Code> {
    use global_hotkey::hotkey::Code;
    Some(match key {
        "Space" => Code::Space,
        "Enter" => Code::Enter,
        "Escape" => Code::Escape,
        "Tab" => Code::Tab,
        "Backspace" => Code::Backspace,
        "Delete" => Code::Delete,
        "Up" => Code::ArrowUp,
        "Down" => Code::ArrowDown,
        "Left" => Code::ArrowLeft,
        "Right" => Code::ArrowRight,
        "Home" => Code::Home,
        "End" => Code::End,
        "PageUp" => Code::PageUp,
        "PageDown" => Code::PageDown,
        "Comma" => Code::Comma,
        "Period" => Code::Period,
        "Slash" => Code::Slash,
        "Backslash" => Code::Backslash,
        "Semicolon" => Code::Semicolon,
        "Apostrophe" => Code::Quote,
        "BracketLeft" => Code::BracketLeft,
        "BracketRight" => Code::BracketRight,
        "Minus" => Code::Minus,
        "Equal" => Code::Equal,
        "Grave" => Code::Backquote,
        other => {
            let c = other.chars().next()?;
            match c {
                'A'..='Z' => match c {
                    'A' => Code::KeyA,
                    'B' => Code::KeyB,
                    'C' => Code::KeyC,
                    'D' => Code::KeyD,
                    'E' => Code::KeyE,
                    'F' => Code::KeyF,
                    'G' => Code::KeyG,
                    'H' => Code::KeyH,
                    'I' => Code::KeyI,
                    'J' => Code::KeyJ,
                    'K' => Code::KeyK,
                    'L' => Code::KeyL,
                    'M' => Code::KeyM,
                    'N' => Code::KeyN,
                    'O' => Code::KeyO,
                    'P' => Code::KeyP,
                    'Q' => Code::KeyQ,
                    'R' => Code::KeyR,
                    'S' => Code::KeyS,
                    'T' => Code::KeyT,
                    'U' => Code::KeyU,
                    'V' => Code::KeyV,
                    'W' => Code::KeyW,
                    'X' => Code::KeyX,
                    'Y' => Code::KeyY,
                    'Z' => Code::KeyZ,
                    _ => return None,
                },
                '0'..='9' => match c {
                    '0' => Code::Digit0,
                    '1' => Code::Digit1,
                    '2' => Code::Digit2,
                    '3' => Code::Digit3,
                    '4' => Code::Digit4,
                    '5' => Code::Digit5,
                    '6' => Code::Digit6,
                    '7' => Code::Digit7,
                    '8' => Code::Digit8,
                    '9' => Code::Digit9,
                    _ => return None,
                },
                _ => return None,
            }
        }
    })
}

// ---- desktop shortcut -----------------------------------------------------

pub const DESKTOP_FILE: &str = "batnotes.desktop";

/// Has the shortcut daemon actually registered our component? This is the
/// difference between "the file is written" and "the key works".
fn kde_component_registered() -> bool {
    use zbus::blocking::connection::Connection;
    let Ok(conn) = Connection::session() else { return false };
    let Ok(iface) = <zbus_names::InterfaceName>::try_from("org.freedesktop.DBus.Introspectable") else {
        return false;
    };
    for component in [KDESKTOP_UUID, "b4a7notes000000000000000000000002"] {
        let path = format!("/component/{component}");
        let Ok(proxy) = zbus::blocking::Proxy::new(
            &conn,
            "org.kde.kglobalaccel",
            path.as_str(),
            iface.clone(),
        ) else {
            return false;
        };
        let r: zbus::Result<String> = proxy.call("Introspect", &());
        if r.is_err() {
            return false;
        }
    }
    true
}
const KDESKTOP_NAMESPACE: &str = "b4a7notes";
const KDESKTOP_UUID: &str = "b4a7notes000000000000000000000001";

/// Register `batnotes --show` as a desktop-level global shortcut.
///
/// On Plasma this is the `[services][<desktop file>]` block that KGlobalAccel
/// reads; the shortcut is then owned by the compositor, which is the only
/// approach guaranteed to receive keys on Wayland regardless of which toolkit
/// the focused client uses.
pub fn install_desktop_shortcut(spec: &HotkeySpec, notes: &HotkeySpec) -> Result<()> {
    let exe = std::env::current_exe()?;
    let capture = format!("{} --show", exe.display());

    let apps = dirs::data_dir()
        .ok_or_else(|| anyhow!("no XDG data dir"))?
        .join("applications");
    std::fs::create_dir_all(&apps)?;
    std::fs::write(
        apps.join(DESKTOP_FILE),
        format!(
            "[Desktop Entry]\nType=Application\nName=Batnotes\nComment=Capture a note\nExec={capture}\nIcon=accessories-text-editor\nTerminal=false\nNoDisplay=true\nStartupNotify=false\nX-GNOME-Autostart-enabled=true\n"
        ),
    )?;

    let autostart = dirs::config_dir()
        .ok_or_else(|| anyhow!("no XDG config dir"))?
        .join("autostart");
    std::fs::create_dir_all(&autostart)?;
    std::fs::write(
        autostart.join(DESKTOP_FILE),
        format!(
            "[Desktop Entry]\nType=Application\nName=Batnotes\nExec={}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
            exe.display()
        ),
    )?;

    install_kde_shortcut(spec, notes)?;
    Ok(())
}

fn install_kde_shortcut(spec: &HotkeySpec, notes: &HotkeySpec) -> Result<()> {
    let path = dirs::config_dir()
        .ok_or_else(|| anyhow!("no XDG config dir"))?
        .join("kglobalshortcutsrc");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let exe = std::env::current_exe()?;
    let capture = format!("{} --show", exe.display());
    let library = format!("{} --show-notes", exe.display());

    // Drop any previous batnotes service blocks so remapping is idempotent.
    let mut out: Vec<String> = Vec::new();
    let mut skipping = false;
    for line in existing.lines() {
        let t = line.trim_start();
        if t.starts_with('[') {
            // Match on the namespace, not one UUID: a newer binary adds
            // components, and an old one must not leave them behind to
            // shadow the new bindings.
            skipping = line.contains(DESKTOP_FILE) || line.contains(KDESKTOP_NAMESPACE);
        }
        if !skipping {
            out.push(line.to_string());
        }
    }
    while out.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        out.pop();
    }

    // Two shortcuts: capture and review. The whole design assumes you can get
    // straight to a blank note, and straight to the list, without navigating.
    for (uuid, name, accel, cmd) in [
        (
            KDESKTOP_UUID,
            "Batnotes capture",
            spec.to_kde_accelerator(),
            capture.as_str(),
        ),
        (
            "b4a7notes000000000000000000000002",
            "Batnotes notes",
            notes.to_kde_accelerator(),
            library.as_str(),
        ),
    ] {
        out.push(format!("[services][{uuid}]"));
        out.push(format!("Name={name}"));
        out.push(format!("Exec={cmd}"));
        out.push(format!("_launch={accel}"));
        out.push(String::new());
    }

    std::fs::write(&path, out.join("\n"))?;
    Ok(())
}

/// Where the single-instance socket lives.
pub fn socket_path() -> PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("batnotes.sock")
}

/// Is another instance listening? This must have no side effects — probing by
/// writing "show" makes the overlay toggle open and straight back shut.
pub fn instance_running() -> bool {
    std::os::unix::net::UnixStream::connect(&socket_path()).is_ok()
}

/// Ask an already-running instance to show its overlay.
pub fn send_show_request(kind: ShowRequest) -> bool {
    use std::io::Write;
    let payload = match kind {
        ShowRequest::Toggle => b"show\n".as_slice(),
        ShowRequest::Library => b"library\n".as_slice(),
    };
    match std::os::unix::net::UnixStream::connect(&socket_path()) {
        Ok(mut s) => s.write_all(payload).is_ok(),
        Err(_) => false,
    }
}

pub fn serve_single_instance() -> Result<Option<std::os::unix::net::UnixListener>> {
    use std::os::unix::net::UnixListener;
    let path = socket_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // A socket left behind by a crashed run would block the bind; remove it
    // only if nobody is actually answering.
    if std::fs::metadata(&path).is_ok() && !instance_running() {
        let _ = std::fs::remove_file(&path);
    }
    if instance_running() {
        // Only signal once, and only because we are genuinely the second
        // instance. `--show-notes` goes straight to the list; `--show` is the
        // capture key.
        let kind = if std::env::args().any(|a| a == "--show-notes") {
            ShowRequest::Library
        } else {
            ShowRequest::Toggle
        };
        send_show_request(kind);
        return Ok(None);
    }
    let listener = UnixListener::bind(&path)?;
    // Non-blocking so the UI thread can poll it without stalling a frame.
    listener.set_nonblocking(true)?;
    Ok(Some(listener))
}

static LISTENER: OnceLock<Mutex<std::os::unix::net::UnixListener>> = OnceLock::new();

pub fn publish_listener(l: std::os::unix::net::UnixListener) {
    let _ = LISTENER.set(Mutex::new(l));
}

/// What a second instance is asking the daemon to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShowRequest {
    /// Capture: open a blank note, or close if already open.
    Toggle,
    /// Review: open the note list.
    Library,
}

/// True when another process asked the overlay to appear. Called once per
/// frame, so it must not block.
pub fn take_show_request() -> Option<ShowRequest> {
    use std::io::Read;
    let Some(cell) = LISTENER.get() else { return None };
    let Ok(l) = cell.lock() else { return None };
    loop {
        match l.accept() {
            Ok((mut stream, _)) => {
                let mut buf = [0u8; 16];
                // Compare only what was actually read. The tail of the buffer
                // stays NUL, and `trim` does not remove NULs, so comparing the
                // whole array silently never matches.
                let n = stream.read(&mut buf).unwrap_or(0);
                match String::from_utf8_lossy(&buf[..n]).trim() {
                    "show" => return Some(ShowRequest::Toggle),
                    "library" => return Some(ShowRequest::Library),
                    _ => {}
                }
            }
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_bindings() {
        assert!(HotkeySpec::parse("Meta+Shift+Space").is_ok());
        assert!(HotkeySpec::parse("Ctrl+S").is_ok());
        assert!(HotkeySpec::parse("Escape").is_ok());
        assert!(HotkeySpec::parse("Alt+Up").is_ok());
        assert!(HotkeySpec::parse("Ctrl+Comma").is_ok());
    }

    #[test]
    fn rejects_nonsense() {
        assert!(HotkeySpec::parse("").is_err());
        assert!(HotkeySpec::parse("Ctrl").is_err());
        assert!(HotkeySpec::parse("Ctrl+NotAKey").is_err());
        assert!(HotkeySpec::parse("A+B").is_err());
    }

    #[test]
    fn normalisation_is_order_and_case_insensitive() {
        let a = HotkeySpec::parse("shift+ctrl+x").unwrap();
        let b = HotkeySpec::parse("Ctrl+Shift+X").unwrap();
        assert_eq!(a.normalised(), b.normalised());
    }

    #[test]
    fn portal_trigger_uses_xdg_syntax() {
        assert_eq!(
            HotkeySpec::parse("Meta+Shift+Space").unwrap().to_portal_trigger(),
            "<Super><Shift>Space"
        );
        assert_eq!(
            HotkeySpec::parse("Ctrl+Enter").unwrap().to_portal_trigger(),
            "<Control>Enter"
        );
    }

    #[test]
    fn unknown_multi_character_key_is_rejected() {
        // "NotAKey" must not silently become "N".
        assert!(HotkeySpec::parse("Ctrl+NotAKey").is_err());
    }

    #[test]
    fn kde_accelerator_uses_kde_names() {
        assert_eq!(
            HotkeySpec::parse("Meta+Shift+Space").unwrap().to_kde_accelerator(),
            "Meta+Shift+Space"
        );
    }

    #[test]
    fn every_default_binding_parses() {
        for (cmd, b) in crate::config::default_bindings() {
            assert!(HotkeySpec::parse(&b).is_ok(), "{cmd:?} = {b:?} does not parse");
        }
    }

    #[test]
    fn every_default_binding_has_an_x11_code() {
        for (cmd, b) in crate::config::default_bindings() {
            let spec = HotkeySpec::parse(&b).unwrap();
            assert!(to_x11_code(&spec.key).is_some(), "{cmd:?} = {b:?} has no x11 code");
        }
    }
}