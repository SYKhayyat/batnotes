# Design — batnotes (Noter)

## 1. Purpose
This document implements the behavior specified in `SRS.md`. It is the build source of truth.

## 2. Architecture

### 2.1 Modules
- `main`: process startup, logging, and runtime.
- `app`: UI state, rendering, and event dispatch.
- `hotkey`: global hotkey lifecycle and action dispatch.
- `config`: settings model, validation, persistence.
- `storage`: note file operations and ordering manifest.
- `search`: query execution and ranking.
- `ui`: chooser, editor, library, and settings views.
- `model`: note, search result, and action types.

### 2.2 Dependency Direction
UI depends on app state. App state depends on storage, search, config, and hotkey. Storage and search do not depend on UI.

## 3. State Machine

```text
Closed
  on invoke -> Chooser

Chooser
  A / choose_note -> Editor(NewDraft)
  B -> Library
  Settings hotkey -> Settings
  Escape / focus loss with no edits -> Closed

Editor(NewDraft)
  Save with text -> storage.save -> Chooser
  Save empty and draft policy enabled -> storage.delete_draft -> Chooser
  Cancel -> Chooser
  focus loss with unsaved -> Confirm(keep/open)

Editor(ExistingNote)
  Save -> storage.save -> Chooser
  Cancel with no changes -> Chooser
  Cancel with changes -> Confirm
  Delete -> ConfirmDelete

Library
  Search changed -> search.update -> Library
  Open note -> Editor(ExistingNote)
  Reorder -> storage.update_order -> Library
  Delete confirmed -> storage.delete -> Library
  Settings hotkey -> Settings
  Back -> Chooser

Settings
  Save/close -> previous view
  Binding conflict -> Settings(error)
```

## 4. Runtime Behavior

### 4.1 Focus Loss
- The overlay tracks whether the editor has unsaved changes.
- On focus loss with no unsaved changes, close immediately.
- On focus loss with unsaved changes, keep overlay and show:
  - Save and close
  - Discard and close
  - Stay open
- This avoids losing capture while remaining non-intrusive.

### 4.2 Overlay Window
- Use a small always-on-top window.
- Position near cursor or configured corner.
- Avoid animations beyond fast fade.
- Restore previous position and size.

## 5. UI Design

### 5.1 Chooser
Layout:
- Title: batnotes
- Two large cards:
  - A — New note
  - B — My notes
- Footer hint: current key bindings.

### 5.2 Editor
Layout:
- Direction selector at top (RTL/LTR).
- Large text area.
- Status line: filename, character count, save state.
- Buttons: Save, Cancel, Delete (existing only).
- Keyboard hints at bottom.

### 5.3 Library
Layout:
- Search box at top.
- Scrollable note cards.
- Each card shows title and at most N configured preview lines.
- Right-side or inline actions: open, move up/down, delete.

### 5.4 Settings
Layout:
- Tabs: General, Keys, Storage.
- General: preview lines, default direction, focus-loss behavior.
- Keys: command list with editable bindings and conflict warnings.
- Storage: notes directory, rebuild index button.

## 6. Data Model

### 6.1 Note
```rust
struct Note {
    id: NoteId,
    path: PathBuf,
    title: String,
    direction: Direction,
    modified_at: DateTime<Utc>,
}
```

### 6.2 Order Manifest
```toml
version = 1
order = ["id1", "id2", "id3"]
```

- Order manifest maps logical IDs to file names.
- Missing IDs are appended in filesystem order.
- Unknown IDs are ignored during load and removed on save.

### 6.3 Configuration
```toml
notes_dir = "~/.batnotes/notes"
preview_lines = 3
default_direction = "rtl"
focus_loss = "auto-hide" # or "prompt"
hotkeys = { invoke = "Alt", choose_a = "Alt-A", choose_b = "Alt-B", save = "Ctrl-S", back = "Escape", settings = "Alt-Comma", delete = "Delete", move_up = "Alt-Up", move_down = "Alt-Down" }
```

## 7. Concurrency

- The UI thread owns editable buffers.
- Search executes in a worker thread.
- Hotkey events use a channel polled by the UI event loop.
- Storage writes occur synchronously after Save because atomic rename keeps latency low.
- Reorder updates only the manifest, not note files.

## 8. Storage and Search

### 8.1 Writes
1. Write note content to temporary file in same directory.
2. fsync temporary file if configured.
3. Rename temporary file over target.
4. Update manifest if identity changed.

### 8.2 Search
- First pass: case-insensitive substring search over title and full text.
- Ranking: title match first, then content match.
- Hebrew and English are both supported through Unicode-aware search.
- Search index is optional in v1; linear scan is acceptable below target counts.
- Rebuild command rescans all note files.

### 8.3 Reordering
- Primary mechanism: drag card in library.
- Keyboard alternatives: move selected note up/down.
- Updates persisted manifest immediately.

## 9. Key Binding System
- Bindings stored as strings.
- Default bindings are seeded on first run.
- Binding parser validates OS-supported combinations.
- Conflicts prevent saving until resolved.
- In-app help always renders the current binding set.

## 10. Error Handling
- Failed save keeps editor open and displays error.
- Failed order manifest write retains in-memory order and retries on next change.
- Corrupt config:
  - Load defaults.
  - Preserve corrupt file as config.toml.bak.
  - Show warning in settings.
- Missing notes directory:
  - Create automatically after user confirmation in settings or first save.

## 11. Testing Strategy

### 11.1 Unit Tests
- Config parsing and conflict detection.
- Note path sanitization.
- Ordering manifest reconciliation.
- Search ranking.

### 11.2 Integration Tests
- Create, edit, save, reopen, delete.
- Reorder persistence.
- Search across content and names.
- RTL/LTR handling.
- Index rebuild from files.

### 11.3 Manual Tests
- Invocation speed.
- Focus-loss behavior.
- Keyboard-only navigation.
- Hebrew editing quality.

## 12. Performance Targets and Measurement
- Measure overlay open with high-resolution timer.
- Benchmark search at 1,000, 5,000, and 10,000 notes.
- Track memory after loading index.
- Reject features that violate latency targets without explicit approval.

## 13. Distribution
- Primary: `cargo install batnotes`.
- Development via `cargo run`.
- No system package in v1.

## 14. Future Extensions
- Optional encrypted notes.
- Optional sync backend.
- Import/export.
- Plugin system.