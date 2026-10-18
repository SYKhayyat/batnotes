use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use crate::model;

#[derive(Debug, Clone)]
pub enum Hit {
    Title,
    Filename,
    Content,
}

#[derive(Debug, Clone)]
pub struct Match {
    pub id: String,
    pub score: u8,
    /// Character offset of the match in the note body, when it was in the body.
    pub at: Option<usize>,
}

/// A snapshot of the library handed to the worker so it can read files itself
/// instead of blocking the UI thread on every keystroke.
#[derive(Clone)]
pub struct Doc {
    pub id: String,
    pub path: PathBuf,
    pub content: Arc<String>,
    pub title: Arc<String>,
    pub direction: model::Direction,
}

impl Doc {
    pub fn new(id: String, path: PathBuf, content: String, direction: model::Direction) -> Doc {
        let title = model::title_of(&content);
        Doc { id, path, content: Arc::new(content), title: Arc::new(title), direction }
    }
}

/// Lowercase a string once, up front. Per-keystroke `to_lowercase()` over every
/// note body is what blows the 150 ms budget at 10k notes.
#[derive(Clone)]
pub struct Indexed {
    docs: Vec<Doc>,
    lowered: Vec<(String, String)>, // (lowercased content, lowercased title)
}

impl Indexed {
    pub fn build(docs: Vec<Doc>) -> Indexed {
        let lowered = docs
            .iter()
            .map(|d| {
                (d.content.to_lowercase(), d.title.to_lowercase())
            })
            .collect();
        Indexed { docs, lowered }
    }

    pub fn len(&self) -> usize {
        self.docs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    pub fn ids(&self) -> Vec<String> {
        self.docs.iter().map(|d| d.id.clone()).collect()
    }

    pub fn get(&self, id: &str) -> Option<&Doc> {
        self.docs.iter().find(|d| d.id == id)
    }

    /// Title match outranks filename, which outranks body — the ranking both
    /// prototypes implemented and the SRS asks for.
    pub fn search(&self, query: &str) -> Vec<Match> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mut hits = Vec::new();
        for (i, d) in self.docs.iter().enumerate() {
            let (content, title) = &self.lowered[i];
            let fname = d.path.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
            if title.contains(&q) {
                hits.push(Match { id: d.id.clone(), score: 3, at: None });
            } else if fname.contains(&q) {
                hits.push(Match { id: d.id.clone(), score: 2, at: None });
            } else if let Some(at) = content.find(&q) {
                hits.push(Match { id: d.id.clone(), score: 1, at: Some(at) });
            }
        }
        hits.sort_by(|a, b| b.score.cmp(&a.score));
        hits
    }

    pub fn preview(&self, id: &str, lines: usize) -> String {
        match self.get(id) {
            Some(d) => preview_of(&d.content, lines),
            None => String::new(),
        }
    }

    /// Everything after the title line.
    ///
    /// A row shows the title as its heading, so a preview that starts at line
    /// one prints the title a second time directly underneath it.
    pub fn preview_body(&self, id: &str, lines: usize) -> String {
        let Some(d) = self.get(id) else { return String::new() };
        let rest = match d.content.lines().find(|l| !l.trim().is_empty()) {
            Some(first) => {
                let at = first.as_ptr() as usize - d.content.as_ptr() as usize;
                &d.content[at + first.len()..]
            }
            None => "",
        };
        preview_of(rest, lines)
    }

    /// A window of the body around the match, so the user can see *why* a note
    /// matched instead of an identical preview for every result.
    pub fn snippet(&self, id: &str, at: usize, before: usize, after: usize) -> String {
        let Some(d) = self.get(id) else { return String::new() };
        let c = &d.content;
        let lo = at.saturating_sub(before);
        let hi = (at + after).min(c.len());
        let mut s = String::new();
        if lo > 0 {
            s.push('…');
        }
        s.push_str(&c[lo..hi].replace('\n', " "));
        if hi < c.len() {
            s.push('…');
        }
        s
    }
}

/// First N non-empty-after-title lines, which is what a reader wants to see.
pub fn preview_of(content: &str, lines: usize) -> String {
    let mut out: Vec<&str> = Vec::new();
    for line in content.lines() {
        if out.is_empty() && line.trim().is_empty() {
            continue;
        }
        if line.trim().is_empty() && out.iter().all(|l| l.trim().is_empty()) {
            continue;
        }
        out.push(line);
        if out.len() >= lines {
            break;
        }
    }
    out.join("\n").trim_end().to_string()
}

// ---- worker ------------------------------------------------------------

pub struct Searcher {
    tx: Sender<Job>,
    rx: Receiver<Vec<Match>>,
    seq: Arc<AtomicU64>,
    latest: Arc<AtomicU64>,
}

/// Debounce: 10k notes is ~150 ms of scanning, and nobody types at 150 ms.
const DEBOUNCE: Duration = Duration::from_millis(120);

enum Job {
    Query { seq: u64, index: Arc<Indexed>, q: String },
    Rescan { #[allow(dead_code)] index: Arc<Indexed> },
}

impl Searcher {
    pub fn new() -> Searcher {
        let (tx, jrx) = mpsc::channel::<Job>();
        let (otx, orx) = mpsc::channel::<Vec<Match>>();
        let seq = Arc::new(AtomicU64::new(0));
        let latest = Arc::new(AtomicU64::new(0));

        let worker_seq = seq.clone();
        let worker_latest = latest.clone();
        std::thread::Builder::new()
            .name("batnotes-search".into())
            .spawn(move || {
                let mut pending: Option<Job> = None;
                loop {
                    match jrx.recv_timeout(DEBOUNCE) {
                        Ok(Job::Rescan { .. }) => {
                            pending = None;
                            worker_latest.store(0, Ordering::SeqCst);
                            let _ = worker_seq;
                        }
                        Ok(Job::Query { seq, index, q }) => {
                            // Coalesce: a burst of keystrokes collapses into one
                            // scan of the final query.
                            pending = Some(Job::Query { seq, index, q });
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if let Some(Job::Query { seq, index, q }) = pending.take() {
                                let hits = index.search(&q);
                                if worker_latest.load(Ordering::SeqCst) == seq {
                                    let _ = otx.send(hits);
                                }
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
            })
            .expect("spawning search thread");

        Searcher { tx, rx: orx, seq, latest }
    }

    pub fn rebuild(&self, index: Arc<Indexed>) {
        let _ = self.tx.send(Job::Rescan { index });
    }

    /// Non-blocking. Returns only results that are still the newest query.
    pub fn submit(&self, index: Arc<Indexed>, query: &str) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.latest.store(seq, Ordering::SeqCst);
        let _ = self.tx.send(Job::Query { seq, index, q: query.to_string() });
    }

    pub fn poll(&self) -> Option<Vec<Match>> {
        self.rx.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx(items: &[(&str, &str)]) -> Indexed {
        Indexed::build(
            items
                .iter()
                .enumerate()
                .map(|(_i, (id, c))| {
                    Doc::new(
                        id.to_string(),
                        PathBuf::from(format!("/n/{id}.txt")),
                        c.to_string(),
                        model::Direction::Ltr,
                    )
                })
                .collect(),
        )
    }

    #[test]
    fn title_beats_body() {
        let i = idx(&[("a", "shopping list\nthe body mentions shabbat once"), ("b", "shabbat\nmore")]);
        let hits = i.search("shabbat");
        assert_eq!(hits[0].id, "b");
        assert_eq!(hits[0].score, 3);
        assert_eq!(hits[1].score, 1);
    }

    #[test]
    fn filename_outranks_body() {
        let i = idx(&[("shabbat-list", "unrelated"), ("x", "groceries\nshabbat")]);
        let hits = i.search("shabbat");
        assert_eq!(hits[0].id, "shabbat-list");
        assert_eq!(hits[0].score, 2);
    }

    #[test]
    fn hebrew_query_works() {
        let i = idx(&[("a", "רשימת קניות לשבת"), ("b", "shopping list")]);
        let hits = i.search("שבת");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "a");
    }

    #[test]
    fn empty_query_returns_nothing() {
        assert!(idx(&[("a", "x")]).search("   ").is_empty());
    }

    #[test]
    fn case_insensitive() {
        let i = idx(&[("a", "Shopping LIST")]);
        assert_eq!(i.search("shopping").len(), 1);
    }

    #[test]
    fn snippet_centres_on_match() {
        let body = format!("a title\n{}{}", "y".repeat(60), "x".repeat(200));
        let i = idx(&[("a", &body)]);
        let hits = i.search(&"x".repeat(5));
        let at = hits[0].at.unwrap();
        let s = i.snippet("a", at, 10, 20);
        assert!(s.starts_with('…'));
        assert!(s.ends_with('…'));
        assert_eq!(s.chars().filter(|c| *c == 'x').count(), 20);
    }

    #[test]
    fn preview_takes_n_lines() {
        assert_eq!(preview_of("a\nb\nc\nd", 2), "a\nb");
    }

    #[test]
    fn preview_skips_leading_blanks() {
        assert_eq!(preview_of("\n\ntitle\nbody", 3), "title\nbody");
    }

    #[test]
    fn ten_thousand_notes_search_under_budget() {
        let docs: Vec<Doc> = (0..10_000)
            .map(|i| {
                Doc::new(
                    format!("n{i}"),
                    PathBuf::from(format!("/n/n{i}.txt")),
                    format!("note {i} lorem ipsum dolor sit amet shabbat torah"),
                    model::Direction::Ltr,
                )
            })
            .collect();
        let i = Indexed::build(docs);
        let start = std::time::Instant::now();
        let hits = i.search("shabbat");
        let elapsed = start.elapsed();
        assert_eq!(hits.len(), 10_000);
        assert!(elapsed < Duration::from_millis(150), "search took {elapsed:?}");
    }
}