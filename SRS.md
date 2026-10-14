# SRS — batnotes (Noter)

## 1. Product
A zero-friction, always-available notes capture and review overlay. The product optimizes for capture without context switching.

## 2. Goals
- Open overlay and begin typing in one gesture.
- Review, search, and reorder notes quickly.
- Stay lightweight and local-only.
- Support Hebrew and English separately (RTL or LTR per note).

## 3. Primary Flows

### 3.1 Capture-first flow
1. User presses global hotkey (default Alt).
2. Overlay shows two large options: A = New Note, B = Notes.
3. Pressing A or clicking it opens a notepad.
4. User types and presses Save/Enter.
5. Overlay returns to hidden state.

### 3.2 Review flow
1. Open overlay and choose B.
2. Library shows note list with configurable preview lines.
3. Search filters by title/content.
4. Selecting a note opens it for editing.
5. Dragging or key shortcuts reorder notes.

### 3.3 Discovery of controls
- Every action displays its current key binding in the UI.
- Bindings are configurable and immediately reflected in hints.

## 4. Functional Requirements

### 4.1 Invocation
- The app can run as a background process without a terminal window.
- Overlay opens on global hotkey.
- Overlay is dismissed on focus loss unless there are unsaved changes.
- With unsaved changes, focus loss shows a non-modal save/discard prompt.
- Dismiss also available via Escape.

### 4.2 Chooser
- F1: Exactly two primary actions are visible initially.
- F2: Each primary action shows title and key hint.
- F3: Choosing an action is possible by mouse or keyboard.
- F4: Default keys: Alt-A and Alt-B.

### 4.3 Editor
- F5: Text editing is plain UTF-8.
- F6: Pressing Save writes the note atomically.
- F7: Note direction is selectable: RTL or LTR.
- F8: Existing text direction is preserved when reopening.
- F9: Cancel returns to chooser without saving.
- F10: Empty note cancellation deletes the draft by default (configurable).

### 4.4 Library
- F11: Notes are listed by user-defined order.
- F12: Each preview shows at most N lines, where N is configurable.
- F13: Search query filters both filenames and note contents.
- F14: Search results update incrementally.
- F15: Reordering is available by drag and drop.
- F16: Reordering is available by keyboard commands.
- F17: Delete requires confirmation.
- F18: Opening a note switches to the editor.

### 4.5 Settings
- F19: Settings can change notes directory.
- F20: Settings can change preview line count.
- F21: Settings can change default text direction.
- F22: All key bindings can be remapped.
- F23: Binding conflicts are detected and reported.
- F24: Settings persist immediately.
- F25: Settings view displays every command and binding.

## 5. Non-Functional Requirements

### 5.1 Performance
- Overlay open latency target: under 50 ms perceived.
- Search over 10,000 notes target: under 150 ms.
- Memory target: under 80 MB resident for 10,000 notes.
- Typing latency target: under 16 ms.

### 5.2 Reliability
- No data loss on crash after a successful atomic write.
- Index corruption recoverable by rebuild.
- Config corruption recovers defaults with a visible warning.

### 5.3 Usability
- Overlay never steals focus unexpectedly beyond opening.
- All primary actions reachable without a mouse.
- Visible, consistent key hints.
- Locale handling for Hebrew filenames.

### 5.4 Security and Privacy
- No network access.
- All files stored under user-configurable directory.
- No command execution from note content.

## 6. Data Requirements
- Notes are plain UTF-8 text with newline preservation.
- Notes use file names safe for all target filesystems.
- Ordering is persisted separately from note content.
- Index is rebuildable from note files.

## 7. Out of Scope for v1
- Sync, encryption, attachments, rich text, spell check.
- Mixed RTL/LTR paragraphs in one note.
- Non-note formats.
- Graphical note export.

## 8. Acceptance Scenarios
- Capture: invoking, typing, saving, and closing happens without losing context.
- Review: 1,000 notes remain searchable and reorderable.
- Hebrew: an RTL note displays and edits correctly without bidi issues in LTR content.
- Recovery: deleting the index rebuilds search without data loss.