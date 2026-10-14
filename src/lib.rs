use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    #[serde(rename = "ltr")]
    Ltr,
    #[serde(rename = "rtl")]
    Rtl,
}

impl Default for Direction {
    fn default() -> Self {
        Direction::Rtl
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    pub direction: Direction,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub modified_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hotkeys {
    pub invoke: String,
    pub choose_a: String,
    pub choose_b: String,
    pub save: String,
    pub back: String,
    pub settings: String,
    pub delete: String,
    pub move_up: String,
    pub move_down: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            invoke: "Alt".to_string(),
            choose_a: "Alt-A".to_string(),
            choose_b: "Alt-B".to_string(),
            save: "Ctrl-S".to_string(),
            back: "Escape".to_string(),
            settings: "Alt-Comma".to_string(),
            delete: "Delete".to_string(),
            move_up: "Alt-Up".to_string(),
            move_down: "Alt-Down".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub notes_dir: PathBuf,
    pub preview_lines: usize,
    pub default_direction: Direction,
    pub focus_loss: String,
    pub hotkeys: Hotkeys,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            notes_dir: dirs_path().join("notes"),
            preview_lines: 3,
            default_direction: Direction::Rtl,
            focus_loss: "auto-hide".to_string(),
            hotkeys: Hotkeys::default(),
        }
    }
}

fn dirs_path() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".batnotes")
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            let cfg = Self::default();
            cfg.save(path)?;
            return Ok(cfg);
        }
        let text = fs::read_to_string(path)?;
        let cfg: Config = toml::from_str(&text)?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        atomic_write(path, text.as_bytes())
    }

    pub fn validate(&self) -> Result<()> {
        if self.preview_lines == 0 {
            return Err(anyhow!("preview_lines must be > 0"));
        }
        if self.focus_loss != "auto-hide" && self.focus_loss != "prompt" {
            return Err(anyhow!("invalid focus_loss"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderManifest {
    pub version: u32,
    pub order: Vec<String>,
}

impl Default for OrderManifest {
    fn default() -> Self {
        Self { version: 1, order: vec![] }
    }
}

pub struct Storage {
    pub notes_dir: PathBuf,
    pub config_dir: PathBuf,
}

impl Storage {
    pub fn new(config: &Config) -> Self {
        let config_dir = config.notes_dir.parent().unwrap_or(&config.notes_dir).to_path_buf();
        Self {
            notes_dir: config.notes_dir.clone(),
            config_dir,
        }
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.notes_dir)?;
        fs::create_dir_all(&self.config_dir)?;
        Ok(())
    }

    pub fn list_notes(&self) -> Result<Vec<PathBuf>> {
        self.ensure_dirs()?;
        let mut out = vec![];
        for entry in fs::read_dir(&self.notes_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("txt") {
                out.push(path);
            }
        }
        Ok(out)
    }

    pub fn save_note(&self, id: &str, content: &str) -> Result<PathBuf> {
        self.ensure_dirs()?;
        let path = self.notes_dir.join(format!("{id}.txt"));
        atomic_write(&path, content.as_bytes())?;
        Ok(path)
    }

    pub fn read_note(&self, path: &Path) -> Result<String> {
        Ok(fs::read_to_string(path)?)
    }

    pub fn delete_note(&self, path: &Path) -> Result<()> {
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.config_dir.join("order.toml")
    }

    pub fn load_manifest(&self) -> Result<OrderManifest> {
        let path = self.manifest_path();
        if !path.exists() {
            return Ok(OrderManifest::default());
        }
        let text = fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn save_manifest(&self, manifest: &OrderManifest) -> Result<()> {
        let path = self.manifest_path();
        let text = toml::to_string_pretty(manifest)?;
        atomic_write(&path, text.as_bytes())
    }

    pub fn ordered_notes(&self, mut paths: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
        let manifest = self.load_manifest()?;
        let by_name: std::collections::BTreeMap<String, PathBuf> =
            paths.drain(..).map(|p| (p.file_stem().unwrap().to_string_lossy().to_string(), p)).collect();
        let mut ordered = vec![];
        for id in &manifest.order {
            if let Some(p) = by_name.get(id) {
                ordered.push(p.clone());
            }
        }
        for (id, p) in by_name {
            if !manifest.order.contains(&id) {
                ordered.push(p);
            }
        }
        Ok(ordered)
    }

    pub fn preview_lines(path: &Path, n: usize) -> Result<String> {
        let text = fs::read_to_string(path)?;
        Ok(text.lines().take(n).collect::<Vec<_>>().join("\n"))
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut tmp = path.to_path_buf();
    tmp.set_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn sanitize_title(title: &str) -> String {
    let mut s = title.trim().to_string();
    if s.is_empty() {
        s = new_id();
    }
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' })
        .collect()
}

pub fn new_id() -> String {
    format!("{}", SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis())
}

pub fn search_notes<'a>(paths: &'a [PathBuf], query: &str) -> Vec<&'a PathBuf> {
    if query.trim().is_empty() {
        return paths.iter().collect();
    }
    let q = query.to_lowercase();
    paths
        .iter()
        .filter(|p| {
            if p.file_name().unwrap().to_string_lossy().to_lowercase().contains(&q) {
                return true;
            }
            if let Ok(text) = fs::read_to_string(p) {
                return text.to_lowercase().contains(&q);
            }
            false
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn config_default_and_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let cfg = Config::default();
        cfg.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.preview_lines, cfg.preview_lines);
    }

    #[test]
    fn atomic_write_and_read() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.txt");
        atomic_write(&path, b"hello").unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "hello");
    }

    #[test]
    fn save_and_order_manifest() {
        let dir = tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.notes_dir = dir.path().join("notes");
        let storage = Storage::new(&cfg);
        storage.ensure_dirs().unwrap();
        let p1 = storage.save_note("one", "a").unwrap();
        let p2 = storage.save_note("two", "b").unwrap();
        let paths = vec![p1.clone(), p2.clone()];
        storage.save_manifest(&OrderManifest { version: 1, order: vec!["two".to_string(), "one".to_string()] }).unwrap();
        let ordered = storage.ordered_notes(paths).unwrap();
        assert_eq!(ordered[0].file_stem().unwrap(), "two");
    }

    #[test]
    fn preview_lines() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("n.txt");
        fs::write(&path, "a\nb\nc\nd").unwrap();
        assert_eq!(Storage::preview_lines(&path, 2).unwrap(), "a\nb");
    }

    #[test]
    fn search_filters() {
        let dir = tempdir().unwrap();
        let p1 = dir.path().join("one.txt");
        let p2 = dir.path().join("two.txt");
        fs::write(&p1, "hello world").unwrap();
        fs::write(&p2, "goodbye").unwrap();
        let paths = vec![p1, p2];
        let res = search_notes(&paths, "hello");
        assert_eq!(res.len(), 1);
    }

    #[test]
    fn sanitize_title_works() {
        assert_eq!(sanitize_title("a/b c!"), "a_b c_");
        assert!(!sanitize_title("").is_empty());
    }

    #[test]
    fn manifest_reconcile_missing() {
        let dir = tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.notes_dir = dir.path().join("notes");
        let storage = Storage::new(&cfg);
        storage.ensure_dirs().unwrap();
        let p1 = storage.save_note("one", "a").unwrap();
        let paths = vec![p1];
        let ordered = storage.ordered_notes(paths.clone()).unwrap();
        assert_eq!(ordered.len(), 1);
    }
}
