# Design — batnotes (Noter)

## 1. Architecture

- `main.rs`: entry point and OS event loop.
- `app.rs`: eframe App implementation and UI state machine.
- `hotkey.rs`: global hotkey registration and mapping to actions.
- `config.rs`: load/save TOML settings.
- `storage.rs`: notes directory, atomic writes, listing.
- `search.rs`: in-memory/SQLite full-text search over notes.
- `ui/chooser.rs`, `ui/editor.rs`, `ui/library.rs`, `ui/settings.rs`.

## 2. State Machine

```text
Closed -> (hotkey) -> Chooser
Chooser -> A -> Editor(new)
Chooser -> B -> Library
Editor -> Save/Cancel -> Chooser
Library -> Open -> Editor(existing)
Library -> Delete -> confirm -> Library
Any view -> Settings hotkey -> Settings -> previous
```

## 3. Data Layout

```text
~/.batnotes/
  config.toml
  notes/
    <timestamp-or-slug>.txt
  index.sqlite (optional, rebuildable)
```

## 4. Concurrency
- UI thread owns all widget state.
- Search runs on worker thread; results sent via channel.
- Hotkey events delivered via channel to UI thread.

## 5. Configuration Schema

```toml
notes_dir = "~/.batnotes/notes"
preview_lines = 3
direction = "rtl" # or ltr
hotkeys = { invoke = "Alt", choose_a = "Alt-A", choose_b = "Alt-B", settings = "Alt-S", back = "Escape" }
```

## 6. Key Bindings Defaults

| Action | Default |
|---|---|
| Invoke overlay | Alt |
| Choose A | Alt-A |
| Choose B | Alt-B |
| Save | Ctrl-S |
| Back | Escape |
| Delete selected | Delete |
| Settings | Alt-, |

All configurable in settings and persisted.

## 7. Storage & Search

- Notes are UTF-8 with BOM-insensitive reads.
- Atomic write via temp file + rename.
- Search scans note contents; first pass linear, optional indexed cache.

## 8. Error Handling

- Missing notes dir: create with user confirmation.
- Failed writes: notify in UI, retain text.
- Corrupt config: fallback defaults and warn.

## 9. Testing

- Unit tests for config parse, storage, search ranking.
- Integration test for create/save/list/delete.
- Manual overlay usability test.

## 10. Security/Privacy

- No network.
- Files only in user directory.
- No eval of note content.
