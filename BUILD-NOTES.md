# batnotes — build notes

## 0. Framework: GTK4, not egui

The UI was originally egui. It was replaced because **egui cannot render
Hebrew correctly**, and Hebrew is a first-class requirement.

egui's text layer maps codepoints through a font's `cmap`. It performs no
shaping and no bidi. For Hebrew that means:

* **No cursive joining** — final and medial forms are never selected, so words
  render as isolated letters.
* **No bidi reordering** — embedded numbers, Latin words and punctuation land
  in the wrong position, so `רשימה 5 פריטים` comes out wrong.

epaint states this outright; the source contains `TODO(emilk): heed bidi
characters`. It is not a missing feature but a missing text engine.

GTK4 was chosen because Pango provides shaping and bidi, and `GtkTextView`
gives cursor, selection, clipboard, undo and IME for free. Everything else —
palette, keycaps, hairline rows, the ruled surface — is CSS now.

**RTL is handled by a real Pango base direction** on the buffer:

```rust
buffer.set_property("direction", gtk4::TextDirection::Rtl);
```

GTK4 removed `pango_attr_direction_new`; base direction is a property, and it
is what actually drives reordering.

### 0.1 What carried over unchanged

The state machine, storage, search, config, hotkey chain and platform glue
never depended on the GUI. `src/app.rs` is deliberately free of widget types,
so the interesting logic is tested without a display (48 unit tests).


`SRS.md` is the product spec. This file records where the build **diverges from
it**, and why. Everything here is a deliberate decision, not a TODO.

## 1. The default invoke key is not `Alt`

SRS §3.1 specified a bare `Alt`. A bare modifier cannot be bound:

* Wayland compositors reserve it.
* On X11 it collides with every `Alt`+key menu.
* The XDG GlobalShortcuts portal requires a key, not a modifier.

Shipped default is **`Meta+Shift+Space`**, remappable in Settings → Keys.

## 2. Global hotkeys are a fallback chain, not one mechanism

On Linux there is no single global-hotkey API that works everywhere, so
`src/hotkey.rs` tries backends in order and **reports which one it actually
got**. It never reports a binding it does not hold — that failure mode is what
the original implementation got wrong.

| Order | Backend | When it is used | Verified on |
|---|---|---|---|
| 1 | `org.freedesktop.portal.GlobalShortcuts` | Portal present and answers | not here (see below) |
| 2 | `XGrabKey` via `global-hotkey` | `XDG_SESSION_TYPE` is `x11`/`xorg` | not here (see below) |
| 3 | Desktop shortcut entry | Always available | written to `kglobalshortcutsrc` |

### 2.1 Why not the portal on this machine

KDE Plasma 6's GlobalShortcuts portal **disconnects the peer during
`CreateSession`**:

```
org.freedesktop.DBus.Error.NoReply: Remote peer disconnected
```

This matches KDE bug 498575. Consequently:

* All nine XDG trigger spellings (`<Super><Shift>space`, `Meta+Shift+Space`,
  `CTRL+ALT+space`, …) return an **empty `trigger_description`**. The binding is
  accepted but the key is never actually assigned.
* The app therefore reports the portal as unavailable and falls through.

The handshake runs **synchronously with a 4 s timeout** for exactly this
reason. Doing it in a background thread and assuming success reproduces the
original bug: the app claims a hotkey while the thread is already dead.

### 2.2 Why `XGrabKey` is refused under Wayland

`global-hotkey` returns `Ok(())` even when its event thread has exited at
startup — the send goes to a dead channel and `let _ =` swallows it. Two
guards were added:

1. `x11-dl` is used to probe the library and display before constructing the
   manager.
2. The session must actually be X11. `DISPLAY` is **not** a session test —
   XWayland sets it on a pure Wayland session, where an X root-window grab
   never receives keystrokes typed into a native Wayland client.

### 2.3 The desktop-shortcut backend

Writes `[services][…]` entries and a `.desktop` file, so the **compositor**
owns the binding — the only approach guaranteed to receive keys on Wayland.

`kglobalaccel` reads that file at startup. Writing it is not the same as the
daemon having read it, so the app queries
`org.kde.kglobalaccel /component/batnotes` and reports:

* registered → "KGlobalAccel is serving Meta+Shift+Space"
* not registered → "written, but the daemon has not reloaded it; log out and
  back in, or restart plasma-kglobalaccel"

Until then the app is still reachable with `batnotes --show`, which signals the
running instance over a Unix socket in `$XDG_RUNTIME_DIR` and exits.

## 3. Notes are `.txt` plus a `.dir` sidecar

SRS §6 assumed ordering lives outside the files, which holds, but it did not
say where **direction** (SRS F7/F8) is stored. A second file per note,
`<name>.dir`, holds `ltr` or `rtl`.

The note file stays exactly the bytes the user typed — the sidecar is
rebuildable, the note is not.

## 4. Search runs off the UI thread

SRS F14 requires incremental results. A synchronous scan of 10,000 notes on
every keystroke blows the 150 ms budget in §5.1 and blocks typing.
`src/search.rs` runs a worker with a 120 ms debounce and a stale-result guard.
Search is case-insensitive and Unicode-aware; ranking is
**title → filename → body**. Verified by
`ten_thousand_notes_search_under_budget`.

## 5. Saves are fsynced when configured

SRS §5.2 promises no data loss after an atomic write. Writing a temp file and
renaming is atomic but *not* durable without `fsync` of the file and its parent
directory. `config.fsync` (default `true`) controls this and is exposed in
Settings → General.

## 6. Window

Frameless, always-on-top, hidden until invoked.

egui cannot blur what is behind a window, so the surface is **opaque warm paper
with a hand-built shadow**, not a translucent blur. This is the one visual
item from the design references that is not achievable without a compositor
protocol.

## 7. Layout

```
src/
  model.rs     Note, Direction, title/filename derivation
  config.rs    Config, Command, Bindings, conflict detection, corrupt-file recovery
  storage.rs   atomic writes, sidecars, order manifest, import/export
  search.rs    worker thread, debounce, ranking, snippets
  hotkey.rs    backend chain, key specs, desktop-shortcut installer, single-instance IPC
  ui/
    mod.rs     Action, View, Tab, key naming — no widget types
    gtk.rs     the GTK4 front end and its CSS
  app.rs       state machine, key dispatch, focus-loss policy
  main.rs      startup, viewport, single instance
```

## 8. Tests

`cargo test` — 48 unit tests covering storage atomicity and sidecars, manifest
reconciliation, search ranking and the 10k budget, binding parsing and
conflict detection, corrupt-config backup, and id uniqueness.

## 9. Portability

### 9.0 Runtime requirement

**Any Linux with GTK4 installed.** That is `libgtk-4-1` / `gtk4` on Debian and
Ubuntu, Fedora, Arch and openSUSE — a standard desktop package, not an exotic
one. Notes stay plain `.txt`; nothing is stored in a GTK format.


The build must run on any Linux, not just this machine. Three environment
differences are handled in `src/platform.rs`; everything else is ordinary code.

### 9.1 Library paths (NixOS)

X11, Wayland and GL libraries live in `/nix/store` and are not on the loader
path. `LD_LIBRARY_PATH` is only read at process start, so the app **re-execs
itself once** with the discovered directories prepended.

Only active when `/nix/store` exists, so no other distribution is affected.
Two bugs were found here and are worth remembering:

* Matching sonames on a bare stem lets `libxcb` be satisfied by
  `libxcb-errors`, which then fails at link time. Matching is on `"{stem}.so"`.
* The re-exec guard has to carry the **next** stage. Passing the current one
  loops forever, re-appending to `LD_LIBRARY_PATH` on every pass until the
  exec fails with `Argument list too long`.

### 9.2 Window backend

winit prefers Wayland whenever `WAYLAND_DISPLAY` is set, which is correct on a
healthy session and fatal on a broken one. The app therefore tries the platform
default first and, only if that fails, **re-execs under XWayland**.

winit can build only one event loop per process, so this cannot be a second
call — it has to be a fresh process. Set `BATNOTES_BACKEND=x11` to pin X11.

### 9.3 Re-exec is the only fallback

Every environment-level fallback is a process restart, not an in-process retry.
At most two restarts happen: one for the loader path, one for the window
backend.