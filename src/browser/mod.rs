pub mod chromium;
pub mod orion;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::NaiveDateTime;
use rusqlite::Connection;
use serde::Serialize;
use tempfile::TempDir;

use crate::config::{BrowserKind, BrowserSettings};

#[derive(Debug, Clone, Serialize)]
pub struct HistoryItem {
    pub id: i64,
    pub url: String,
    pub title: Option<String>,
    pub host: String,
    pub last_visit_time: NaiveDateTime,
    pub visit_count: i64,
}

/// RAII wrapper around a temp-dir-backed copy of the browser DB.
pub struct HistorySnapshot {
    path: PathBuf,
    _dir: TempDir,
}

impl HistorySnapshot {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub trait BrowserHistory: Send + Sync {
    fn recent(&self, conn: &Connection, limit: u32) -> anyhow::Result<Vec<HistoryItem>>;
    fn top_sites(&self, conn: &Connection, limit: u32) -> anyhow::Result<Vec<HistoryItem>>;
    /// Returns (url, title) pairs from the browser's reading/bookmark list.
    /// Default: empty — only Orion overrides this.
    fn reading_list_items(&self, _history_db_path: &Path) -> Vec<(String, String)> {
        vec![]
    }
}

/// Copies the DB and any WAL/SHM sidecar files into a fresh temp directory
/// to avoid lock conflicts with a running browser process.
pub fn copy_db(source: &Path) -> std::io::Result<HistorySnapshot> {
    let dir = TempDir::new()?;
    let dest = dir.path().join("history");
    std::fs::copy(source, &dest)?;
    for suffix in ["-wal", "-shm"] {
        let mut src_side = source.as_os_str().to_os_string();
        src_side.push(suffix);
        let mut dst_side = dest.as_os_str().to_os_string();
        dst_side.push(suffix);
        match std::fs::copy(Path::new(&src_side), Path::new(&dst_side)) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(HistorySnapshot { path: dest, _dir: dir })
}

/// Returns the right adapter based on `settings.kind`.
/// Chrome, Brave, Arc, and Edge all share the Chromium schema.
pub fn for_config(settings: &BrowserSettings) -> Arc<dyn BrowserHistory> {
    match settings.kind {
        BrowserKind::Chromium | BrowserKind::Chrome | BrowserKind::Brave
        | BrowserKind::Arc | BrowserKind::Edge => Arc::new(chromium::ChromiumBrowser),
        BrowserKind::Orion => Arc::new(orion::OrionBrowser),
    }
}
