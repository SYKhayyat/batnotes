# SRS — batnotes (Noter)

## 1. Purpose
A lightweight, always-available notes overlay for Linux (later other OSes). One invocation opens a two-option chooser; each option opens either a notepad or the notes library.

## 2. Users
- Daily capture user who needs distraction-free quick notes.
- Power user who wants keyboard-only workflows and configurable bindings.
- Hebrew/LTR users who require correct paragraph direction per note.

## 3. Functional Requirements

### 3.1 Invocation
- F1: Global hotkey (default Alt) opens overlay in <100 ms perceived.
- F2: Overlay shows exactly two primary actions initially.
- F3: Action A opens note editor; Action B opens note library.
- F4: Actions selectable by configured keys (default Alt-A / Alt-B).

### 3.2 Editor
- F5: Plain text editing with save to .txt.
- F6: Auto-create note if empty? configurable.
- F7: Configurable direction per note/buffer (RTL/LTR) not mixed.

### 3.3 Library
- F8: List notes with preview of first N configurable lines.
- F9: Search notes by filename/content.
- F10: Delete note with confirmation.
- F11: Open note in editor.
- F12: Settings view from keyboard and maybe menu.

### 3.4 Settings
- F13: Key bindings fully configurable and visible in UI.
- F14: Preview line count configurable.
- F15: Notes directory configurable.
- F16: Direction preference configurable.

### 3.5 Storage
- F17: Notes stored as plain UTF-8 .txt.
- F18: Atomic writes; no data loss on crash.
- F19: Search index stored alongside notes; rebuildable.

## 4. Non-Functional Requirements
- N1: Fast startup, low memory.
- N2: No network.
- N3: Cross-platform target (Linux first).
- N4: Accessibility for keyboard navigation.
- N5: No accidental data overwrite.

## 5. Constraints
- Linux first.
- Plain text only.
- User controls all key bindings.

## 6. Out of Scope (v1)
- Rich text, images, sync, collaboration.
- Mixed directional runs.
