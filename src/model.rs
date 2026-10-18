use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Ltr,
    Rtl,
}

impl Default for Direction {
    fn default() -> Self {
        Direction::Rtl
    }
}

impl Direction {
    pub fn other(self) -> Direction {
        match self {
            Direction::Ltr => Direction::Rtl,
            Direction::Rtl => Direction::Ltr,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Direction::Ltr => "LTR",
            Direction::Rtl => "RTL",
        }
    }

    /// BCP-47 tag handed to the text layer so Hebrew is shaped and
    /// bidirectional correctly instead of being guessed at from codepoints.
    pub fn lang(self) -> &'static str {
        match self {
            Direction::Ltr => "en",
            Direction::Rtl => "he",
        }
    }
}

/// A note as it exists on disk: one `.txt` file plus one `.dir` sidecar.
#[derive(Debug, Clone)]
pub struct Note {
    pub id: String,
    pub path: PathBuf,
    pub direction: Direction,
    pub modified_at: std::time::SystemTime,
    pub size: u64,
}

impl Note {
    pub fn read(&self) -> std::io::Result<String> {
        std::fs::read_to_string(&self.path)
    }
}

/// Which way a note reads, decided by its first strongly-directional character.
///
/// Only used when the `.dir` sidecar is missing — an imported file, or one
/// dropped into the folder by hand. Defaulting those to RTL (which is what
/// `Direction::default` does) laid every English note out right-to-left and
/// made the chooser's Hebrew count a lie.
pub fn infer_direction(content: &str) -> Direction {
    for c in content.chars() {
        match c {
            // Hebrew, Arabic and the rest of their ranges.
            '\u{0590}'..='\u{08ff}' | '\u{fb1d}'..='\u{fdff}' | '\u{fe70}'..='\u{feff}' => {
                return Direction::Rtl
            }
            // Latin, and anything else that reads left to right.
            _ if c.is_alphabetic() => return Direction::Ltr,
            // Punctuation, digits and whitespace say nothing.
            _ => continue,
        }
    }
    Direction::Ltr
}

/// The first non-blank line, used as the human title everywhere in the UI.
pub fn title_of(content: &str) -> String {
    content
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("Untitled note")
        .chars()
        .take(64)
        .collect()
}

/// Title -> filename. Mirrors the behaviour of both prototypes: lowercase,
/// hyphenated, stripped of characters that are unsafe on any target filesystem.
pub fn filename_of(content: &str) -> String {
    let t = title_of(content).to_lowercase();
    let mut out = String::new();
    let mut last_dash = true;
    for c in t.chars() {
        if c.is_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if c.is_whitespace() || c == '-' || c == '_' {
            if !last_dash {
                out.push('-');
                last_dash = true;
            }
        }
        // everything else is dropped rather than replaced, so Hebrew and
        // punctuation do not turn into runs of underscores.
    }
    while out.ends_with('-') {
        out.pop();
    }
    out.truncate(48);
    if out.is_empty() {
        "untitled".to_string()
    } else {
        out
    }
}

pub fn is_hebrew(s: &str) -> bool {
    s.chars().any(|c| ('\u{0590}'..='\u{05FF}').contains(&c))
}

/// Time-ordered, collision-resistant, and monotonic within a process so two
/// notes saved in the same millisecond cannot collide.
pub fn new_id() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // Nanoseconds plus a process-wide counter: unique even when several notes
    // are saved inside the same millisecond, which an 8-bit counter was not.
    format!("{:x}-{:x}", ms, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_is_read_off_the_first_strong_character() {
        assert_eq!(infer_direction("שלום עולם"), Direction::Rtl);
        assert_eq!(infer_direction("hello world"), Direction::Ltr);
        // Leading punctuation, digits and spaces carry no direction.
        assert_eq!(infer_direction("  \n\t-- 14.3.2026: שלום"), Direction::Rtl);
        assert_eq!(infer_direction("2026: hello"), Direction::Ltr);
        // Nothing directional at all: do not claim it is Hebrew.
        assert_eq!(infer_direction("123 -- ..."), Direction::Ltr);
        assert_eq!(infer_direction(""), Direction::Ltr);
    }

    #[test]
    fn title_is_the_first_line() {
        assert_eq!(title_of("first\nsecond"), "first");
        assert_eq!(title_of("\n\n  spaced  \nrest"), "spaced");
        assert_eq!(title_of(""), "Untitled note");
    }
}

