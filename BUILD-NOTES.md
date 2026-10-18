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

### 6.1 The app owns every colour, including a dark palette

On a dark desktop session (KDE with `breeze-dark`) GTK hands the application the
*dark* variant of the theme, so anything the app did not style came back in the
desktop's colours. The notes list was unstyled and rendered near-black — dark
green Hebrew on `#242424` — inside an otherwise cream window.

The stylesheet now names every widget the app uses: `list`, `entry`,
`textview` *and its inner `text` node*, `scrolledwindow`, `undershoot`,
`separator`, `switch` and `scrollbar`. The `textview > text` node is the one
that matters most: styling only the `textview` leaves the writing surface to
the theme.

`config.theme` picks the palette (`light` or `dark`, default `light`) and
Settings → General switches it. It is deliberately independent of the desktop
setting — the window looks the same either way. In dark mode the accent is a
pale green, so ink drawn on top of it flips to dark.

### 6.2 Labels have to be centred, or they become tall empty pills

A `GtkLabel` stretches to the height of the box it is in. Every keycap, the
chooser's icon tile and the Record buttons therefore rendered as 40–50px tall
empty rectangles. They are centred at construction time, in `keycap()`, so
every call site inherits the fix.

The ruled-paper gradient behind the editor is gone. It was a fixed 35px
`repeating-linear-gradient` while the real line height is about 20px, so the
rules drifted away from the text — by the fifth line no rule sat near the
baseline at all.

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

## 7.1 What the library actually does now

The list was a picture of your notes rather than a way into them. Nothing in the
GTK layer dispatched `OpenNote`, `Select` or `Move`, so clicking a row did
nothing, `app.selected` never moved, the footer's Delete always targeted the
first note, and the ↑/↓ buttons moved an index that nothing displayed.
`ReorderFocus` only changed `selected` and never called `move_note`, so
reordering was unreachable even from the keyboard.

It now follows the plainest model: click or Enter opens, arrows move the
selection, Alt+arrows reorder. Arrow keys and Enter are GTK's own listbox
behaviour; the bindings map to `Action::MoveSelected`.

`resync_rows` replaces `recompute_rows` for everything except changing the
query. `recompute_rows` resets a filtered list to every note, so reordering,
saving and deleting each turned a search back into the full library.

The footer legend used to read "Del delete" and "Ctrl , settings": a key
pasted into the label text, where the reader cannot tell the action from its
shortcut. Action buttons now carry a separate key chip, and the legend only
lists bindings that have no button — which is why it is now one or two items
instead of eight.

## 7.2 Settings had one tab, not three

`build_settings` rendered the General page and nothing else. `keys_page` and
`storage_page` existed and were unreachable, so key remapping, the notes
directory, index rebuild and "open folder" were all absent from the UI — and so
was the global-hotkey status, which is the one place that tells you KDE never
actually granted the key. All three tabs are reachable now, and the segmented
controls replaced `Stack` widgets that displayed only the option already in
effect.

## 7.3 Direction without a sidecar

`Direction::default()` is `Rtl`, so any note with no `.dir` sidecar — an
import, or a file dropped into the folder by hand — was laid out
right-to-left. English notes rendered mirrored, and the chooser's "N Hebrew"
count counted them. `model::infer_direction` reads the first strongly
directional character instead. The sidecar still wins when it exists.

Per-row direction in the list is applied to the note's **text block**, not to
the row: the title, preview and timestamp take the note's alignment, while the
list order, scrollbar and delete button stay put so the list still reads as
one list.

## 7.4 Errors and warnings that nothing displayed

`App::storage_error` and `App::config_warning` were written by the state machine
and read by no widget, so a failed save was indistinguishable from a successful
one. Both are now banners above the view, with a dismiss action. A save error
also stops the overlay closing, because closing implies it was captured.

## 7.5 Two smaller ones

* `Ui::signature` fingerprinted `rows.len()` but not the query, so searching
  for something with three hits and then something else with three hits left
  the first results on screen. It now includes the query, the selection, the
  settings tab and the notice state.
* A signal fired *during* a rebuild (the search box's own `set_text`) started a
  second rebuild inside the first, and the two fought over the same widgets —
  GTK rejected the appends and the view came out half built. Rebuilds are now
  non-reentrant, and the search handler ignores a value it already has.

## 8. Tests

`cargo test` — 64 unit tests covering storage atomicity and sidecars, manifest
reconciliation, search ranking and the 10k budget, binding parsing and
conflict detection, corrupt-config backup, id uniqueness, direction inference,
reorder persistence, and a filtered list surviving a save.

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