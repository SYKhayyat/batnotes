use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::model::{self, Direction, Note};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderManifest {
    pub version: u32,
    pub order: Vec<String>,
}

impl Default for OrderManifest {
    fn default() -> Self {
        OrderManifest { version: 1, order: vec![] }
    }
}

pub struct Storage {
    pub notes_dir: PathBuf,
    pub config_dir: PathBuf,
    fsync: bool,
}

impl Storage {
    pub fn new(config: &Config) -> Self {
        let notes_dir = config.notes_dir.clone();
        let config_dir = notes_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| notes_dir.clone());
        Storage { notes_dir, config_dir, fsync: config.fsync }
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.notes_dir)?;
        Ok(())
    }

    // ---- listing -------------------------------------------------------

    pub fn list_notes(&self) -> Result<Vec<Note>> {
        self.ensure_dirs()?;
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.notes_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("txt") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_string_lossy().into_owned().into()) else {
                continue;
            };
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let direction = self.read_direction(&path);
            out.push(Note {
                id,
                path,
                direction,
                modified_at: meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                size: meta.len(),
            });
        }
        Ok(out)
    }

    /// Manifest order first, then anything unlisted in filename order, so the
    /// list is deterministic even with no manifest at all.
    pub fn ordered(&self, notes: Vec<Note>) -> Result<Vec<Note>> {
        let manifest = self.load_manifest()?;
        let mut by_id: BTreeMap<String, Note> =
            notes.into_iter().map(|n| (n.id.clone(), n)).collect();
        let mut ordered = Vec::with_capacity(by_id.len());
        for id in &manifest.order {
            if let Some(n) = by_id.remove(id) {
                ordered.push(n);
            }
        }
        ordered.extend(by_id.into_values());
        Ok(ordered)
    }

    // ---- direction sidecar --------------------------------------------

    /// Stored next to the note as `<name>.dir` so the note file itself stays
    /// exactly the bytes the user typed. Deleting the sidecar costs you the
    /// direction, never the text.
    fn sidecar_path(&self, note_path: &Path) -> PathBuf {
        note_path.with_extension("dir")
    }

    fn read_direction(&self, note_path: &Path) -> Direction {
        fs::read_to_string(self.sidecar_path(note_path))
            .ok()
            .and_then(|s| match s.trim() {
                "ltr" => Some(Direction::Ltr),
                "rtl" => Some(Direction::Rtl),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn write_direction(&self, note_path: &Path, d: Direction) -> Result<()> {
        atomic_write(
            &self.sidecar_path(note_path),
            match d {
                Direction::Ltr => b"ltr",
                Direction::Rtl => b"rtl",
            },
            self.fsync,
        )
    }

    // ---- reading / writing --------------------------------------------

    pub fn read_note(&self, path: &Path) -> Result<String> {
        Ok(fs::read_to_string(path)?)
    }

    pub fn save(&self, id: &str, content: &str, direction: Direction) -> Result<Note> {
        self.ensure_dirs()?;
        let path = self.notes_dir.join(format!("{id}.txt"));
        atomic_write(&path, content.as_bytes(), self.fsync)?;
        self.write_direction(&path, direction)?;
        let meta = fs::metadata(&path)?;
        Ok(Note {
            id: id.to_string(),
            path,
            direction,
            modified_at: meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH),
            size: meta.len(),
        })
    }

    pub fn delete(&self, note: &Note) -> Result<()> {
        if note.path.exists() {
            fs::remove_file(&note.path)?;
        }
        let side = self.sidecar_path(&note.path);
        if side.exists() {
            let _ = fs::remove_file(side);
        }
        Ok(())
    }

    pub fn rename_to_match_content(&self, note: &Note, content: &str) -> Result<Note> {
        let want = format!("{}.txt", model::filename_of(content));
        let current = note
            .path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if want == current {
            return Ok(note.clone());
        }
        let target = self.notes_dir.join(&want);
        if target.exists() {
            // A different note already owns that name; keep ours as-is rather
            // than silently merging two notes.
            return Ok(note.clone());
        }
        fs::rename(&note.path, &target)?;
        let _ = fs::remove_file(self.sidecar_path(&note.path));
        self.write_direction(&target, note.direction)?;
        Ok(Note { path: target, ..note.clone() })
    }

    /// Copy an external `.txt` into the library, returning the new note.
    pub fn import_file(&self, src: &Path) -> Result<Note> {
        let content = fs::read_to_string(src).context("reading imported file")?;
        let id = model::new_id();
        self.save(&id, &content, self.read_direction(src))
    }

    /// Plain-text export target.
    pub fn export_path(&self, content: &str) -> PathBuf {
        dirs::download_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("{}.txt", model::filename_of(content)))
    }

    // ---- order manifest -------------------------------------------------

    pub fn manifest_path(&self) -> PathBuf {
        self.config_dir.join("order.toml")
    }

    pub fn load_manifest(&self) -> Result<OrderManifest> {
        let path = self.manifest_path();
        if !path.exists() {
            return Ok(OrderManifest::default());
        }
        match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).or(Ok(OrderManifest::default())),
            Err(_) => Ok(OrderManifest::default()),
        }
    }

    pub fn save_manifest(&self, manifest: &OrderManifest) -> Result<()> {
        atomic_write(&self.manifest_path(), toml::to_string_pretty(manifest)?.as_bytes(), self.fsync)
    }

    pub fn write_order(&self, order: &[String]) -> Result<()> {
        self.save_manifest(&OrderManifest { version: 1, order: order.to_vec() })
    }
}

/// Write to a sibling temp file, fsync it, then rename over the target.
/// Rename within a directory is atomic, so a reader either sees the whole old
/// file or the whole new one.
pub fn atomic_write(path: &Path, bytes: &[u8], fsync: bool) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let stem = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "out".into());
    let tmp = path.with_file_name(format!(".{stem}.tmp"));
    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.flush()?;
        if fsync {
            // Without this the rename can be durable while the contents are
            // not, which is exactly the "no data loss on crash" requirement.
            f.sync_all()?;
        }
    }
    fs::rename(&tmp, path)?;
    if let Some(parent) = path.parent() {
        if fsync {
            if let Ok(d) = File::open(parent) {
                let _ = d.sync_all();
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn storage(dir: &Path) -> Storage {
        let mut cfg = Config::default();
        cfg.notes_dir = dir.join("notes");
        let s = Storage::new(&cfg);
        s.ensure_dirs().unwrap();
        s
    }

    #[test]
    fn save_read_preserves_content_and_direction() {
        let d = tempfile::tempdir().unwrap();
        let s = storage(d.path());
        s.save("a", "שלום\nעולם", Direction::Rtl).unwrap();
        let notes = s.list_notes().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].direction, Direction::Rtl);
        assert_eq!(s.read_note(&notes[0].path).unwrap(), "שלום\nעולם");
    }

    #[test]
    fn sidecar_is_written_but_is_not_listed_as_a_note() {
        let d = tempfile::tempdir().unwrap();
        let s = storage(d.path());
        s.save("a", "x", Direction::Ltr).unwrap();
        let files: Vec<_> = fs::read_dir(d.path().join("notes")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert!(files.iter().any(|f| f == "a.txt"));
        // The sidecar exists on disk but must never be listed as a note.
        assert!(files.iter().any(|f| f == "a.dir"));
        assert_eq!(s.list_notes().unwrap().len(), 1);
    }

    #[test]
    fn manifest_order_wins() {
        let d = tempfile::tempdir().unwrap();
        let s = storage(d.path());
        s.save("one", "a", Direction::Ltr).unwrap();
        s.save("two", "b", Direction::Ltr).unwrap();
        s.write_order(&["two".into(), "one".into()]).unwrap();
        let ordered = s.ordered(s.list_notes().unwrap()).unwrap();
        assert_eq!(ordered[0].id, "two");
        assert_eq!(ordered[1].id, "one");
    }

    #[test]
    fn unlisted_notes_are_appended() {
        let d = tempfile::tempdir().unwrap();
        let s = storage(d.path());
        s.save("one", "a", Direction::Ltr).unwrap();
        s.save("two", "b", Direction::Ltr).unwrap();
        s.write_order(&["two".into()]).unwrap();
        let ordered = s.ordered(s.list_notes().unwrap()).unwrap();
        assert_eq!(ordered.len(), 2);
        assert_eq!(ordered[0].id, "two");
    }

    #[test]
    fn delete_removes_both_files() {
        let d = tempfile::tempdir().unwrap();
        let s = storage(d.path());
        let n = s.save("a", "x", Direction::Ltr).unwrap();
        s.delete(&n).unwrap();
        assert!(!n.path.exists());
        assert!(!s.sidecar_path(&n.path).exists());
    }

    #[test]
    fn atomic_write_leaves_no_temp_behind() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.txt");
        atomic_write(&p, b"hello", true).unwrap();
        let strays: Vec<_> = fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("tmp"))
            .collect();
        assert!(strays.is_empty(), "left temp files: {strays:?}");
    }

    #[test]
    fn rename_avoids_collisions() {
        let d = tempfile::tempdir().unwrap();
        let s = storage(d.path());
        let a = s.save("aaa", "same title", Direction::Ltr).unwrap();
        let b = s.save("bbb", "same title", Direction::Ltr).unwrap();
        let renamed = s.rename_to_match_content(&a, "same title").unwrap();
        assert_ne!(renamed.path, b.path);
        assert!(renamed.path.exists());
    }

    #[test]
    fn new_ids_do_not_collide() {
        let set: std::collections::HashSet<_> = (0..1000).map(|_| model::new_id()).collect();
        assert_eq!(set.len(), 1000);
    }
}