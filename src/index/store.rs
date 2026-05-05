use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json;
use thiserror::Error;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchStatus {
    Pending,
    Fetched,
    AuthWall,
    Skip,
    Error,
}

impl FetchStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Fetched => "fetched",
            Self::AuthWall => "auth_wall",
            Self::Skip => "skip",
            Self::Error => "error",
        }
    }
}

impl rusqlite::types::ToSql for FetchStatus {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(self.as_str().into())
    }
}

impl rusqlite::types::FromSql for FetchStatus {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        let s = <String as rusqlite::types::FromSql>::column_result(value)?;
        match s.as_str() {
            "pending" => Ok(Self::Pending),
            "fetched" => Ok(Self::Fetched),
            "auth_wall" => Ok(Self::AuthWall),
            "skip" => Ok(Self::Skip),
            "error" => Ok(Self::Error),
            other => Err(rusqlite::types::FromSqlError::Other(
                format!("unknown fetch_status: {other}").into(),
            )),
        }
    }
}

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, IndexError>;

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub rank: f64,
    pub first_visit_at: Option<String>,
    pub last_visit_at: Option<String>,
    pub starred: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PageEntry {
    pub url: String,
    pub title: String,
    pub fetch_status: FetchStatus,
    pub starred: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WeeklyEntry {
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub host: String,
    pub last_visit_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VectorResult {
    pub url: String,
    pub title: String,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub total_pages: u64,
    pub fetched: u64,
    pub pending: u64,
    pub embedded: u64,
    pub auth_wall: u64,
    pub skipped: u64,
    pub favicons: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportPage {
    pub url: String,
    pub title: String,
    pub body: String,
    pub starred: bool,
    pub first_visit_at: Option<String>,
    pub last_visit_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullExport {
    pub version: u32,
    pub exported_at: String,
    pub pages: Vec<ExportPage>,
    pub ban_list: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct IndexStore {
    path: PathBuf,
}

impl IndexStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // If the DB itself was deleted but the WAL/SHM side-files were left behind,
        // SQLite will corrupt the new empty DB by replaying the stale WAL.
        if !path.exists() {
            info!(db = %path.display(), "index DB not found, starting fresh");
            for suffix in ["-wal", "-shm"] {
                let mut side = path.as_os_str().to_os_string();
                side.push(suffix);
                let side_path = std::path::Path::new(&side);
                if side_path.exists() {
                    warn!(file = %side_path.display(), "removing orphaned WAL/SHM file");
                    let _ = std::fs::remove_file(side_path);
                }
            }
        }
        debug!(db = %path.display(), "opening index DB");
        let conn = Connection::open(path)?;
        debug!("running schema migrations");
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS pages (
                 id           INTEGER PRIMARY KEY AUTOINCREMENT,
                 url          TEXT UNIQUE NOT NULL,
                 title        TEXT NOT NULL DEFAULT '',
                 body         TEXT NOT NULL DEFAULT '',
                 fetched_at   TIMESTAMP,
                 fetch_status TEXT NOT NULL DEFAULT 'pending'
             );
             CREATE VIRTUAL TABLE IF NOT EXISTS pages_fts USING fts5(title, body);
             CREATE TABLE IF NOT EXISTS favicons (
                 host       TEXT PRIMARY KEY,
                 mime       TEXT NOT NULL DEFAULT 'image/x-icon',
                 data       BLOB NOT NULL,
                 fetched_at TIMESTAMP
             );",
        )?;
        let _ = conn.execute("ALTER TABLE pages ADD COLUMN embedding BLOB", []);
        let _ = conn.execute("ALTER TABLE pages ADD COLUMN last_visit_at TIMESTAMP", []);
        let _ = conn.execute(
            "ALTER TABLE pages ADD COLUMN fetch_attempts INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE pages ADD COLUMN starred INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute("ALTER TABLE pages ADD COLUMN first_visit_at TIMESTAMP", []);
        let _ = conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS banned_hosts (host TEXT PRIMARY KEY);
             CREATE TABLE IF NOT EXISTS cluster_ignored_domains (domain TEXT PRIMARY KEY);
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);",
        );
        // Migrate from external-content FTS to standalone. External-content FTS
        // requires passing old column values on delete, which desynchronises under
        // any interrupted write. Standalone FTS stores content internally so a
        // plain DELETE works without needing old values.
        let content_setting: String = conn
            .query_row(
                "SELECT COALESCE(v, '') FROM pages_fts_config WHERE k = 'content'",
                [],
                |r| r.get(0),
            )
            .unwrap_or_default();
        if !content_setting.is_empty() {
            warn!("migrating FTS index from external-content to standalone");
            conn.execute_batch("DROP TABLE IF EXISTS pages_fts")?;
            conn.execute_batch("CREATE VIRTUAL TABLE pages_fts USING fts5(title, body)")?;
            let n: u64 = conn.query_row(
                "SELECT COUNT(*) FROM pages WHERE fetch_status = 'fetched'",
                [],
                |r| r.get(0),
            )?;
            conn.execute_batch(
                "INSERT INTO pages_fts(rowid, title, body)
                 SELECT id, title, body FROM pages WHERE fetch_status = 'fetched'",
            )?;
            info!(rows = n, "FTS migration complete");
        }
        debug!("checking DB integrity");
        let first_issue: String = conn.query_row("PRAGMA integrity_check(1)", [], |r| r.get(0))?;
        if first_issue != "ok" {
            warn!(issue = %first_issue, "integrity check failed — dropping and rebuilding FTS index");
            conn.execute_batch(
                "DROP TABLE IF EXISTS pages_fts;
                 CREATE VIRTUAL TABLE pages_fts USING fts5(
                     title, body, content=pages, content_rowid=id
                 );
                 INSERT INTO pages_fts(pages_fts) VALUES('rebuild');",
            )?;
            info!("FTS index rebuilt");
        }
        debug!("index ready");
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    pub fn upsert_page(&self, url: &str, title: &str, body: &str) -> Result<()> {
        let mut conn = Connection::open(&self.path)?;
        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let tx = conn.transaction()?;

        let existing: Option<i64> = tx
            .query_row("SELECT id FROM pages WHERE url = ?1", [url], |row| {
                row.get(0)
            })
            .optional()?;

        if let Some(id) = existing {
            tx.execute("DELETE FROM pages_fts WHERE rowid = ?1", [id])?;
            tx.execute(
                "UPDATE pages SET title=?1, body=?2, fetched_at=?3, fetch_status='fetched' WHERE id=?4",
                params![title, body, now, id],
            )?;
            tx.execute(
                "INSERT INTO pages_fts(rowid, title, body) VALUES(?1, ?2, ?3)",
                params![id, title, body],
            )?;
        } else {
            tx.execute(
                "INSERT INTO pages(url, title, body, fetched_at, fetch_status) \
                 VALUES(?1, ?2, ?3, ?4, 'fetched')",
                params![url, title, body, now],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO pages_fts(rowid, title, body) VALUES(?1, ?2, ?3)",
                params![id, title, body],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    pub fn register_urls<'a>(&self, urls: impl Iterator<Item = &'a str>) -> Result<()> {
        let conn = Connection::open(&self.path)?;
        let mut stmt = conn.prepare("INSERT OR IGNORE INTO pages(url) VALUES(?1)")?;
        for url in urls {
            stmt.execute([url])?;
        }
        Ok(())
    }

    /// Upsert visit records. For each URL:
    /// - if new, insert with status='pending' and the given visit timestamp.
    /// - if existing and the new visit is more recent, update last_visit_at and
    ///   reset fetch_status to 'pending' so the page is re-fetched (unless it
    ///   was previously skipped as non-HTML content).
    pub fn register_visits<'a>(
        &self,
        visits: impl Iterator<Item = (&'a str, &'a str)>,
    ) -> Result<()> {
        let conn = Connection::open(&self.path)?;
        let mut stmt = conn.prepare(
            "INSERT INTO pages(url, first_visit_at, last_visit_at) VALUES(?1, ?2, ?2)
             ON CONFLICT(url) DO UPDATE SET
               first_visit_at = COALESCE(first_visit_at, excluded.first_visit_at),
               last_visit_at = CASE
                 WHEN excluded.last_visit_at > COALESCE(last_visit_at, '')
                 THEN excluded.last_visit_at
                 ELSE last_visit_at
               END,
               fetch_status = CASE
                 WHEN excluded.last_visit_at > COALESCE(last_visit_at, '')
                      AND fetch_status != 'skip'
                 THEN 'pending'
                 ELSE fetch_status
               END",
        )?;
        let patterns: Vec<String> = conn
            .prepare("SELECT host FROM banned_hosts")?
            .query_map([], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        for (url, visit_at) in visits {
            if !patterns
                .iter()
                .any(|p| crate::config::matches_ban_pattern(url, p))
            {
                stmt.execute([url, visit_at])?;
            }
        }
        Ok(())
    }

    pub fn stats(&self) -> Result<Stats> {
        let conn = Connection::open(&self.path)?;
        let count = |sql: &str| -> rusqlite::Result<u64> { conn.query_row(sql, [], |r| r.get(0)) };
        Ok(Stats {
            total_pages: count("SELECT COUNT(*) FROM pages")?,
            fetched: count("SELECT COUNT(*) FROM pages WHERE fetch_status = 'fetched'")?,
            pending: count("SELECT COUNT(*) FROM pages WHERE fetch_status = 'pending'")?,
            embedded: count("SELECT COUNT(*) FROM pages WHERE embedding IS NOT NULL")?,
            auth_wall: count("SELECT COUNT(*) FROM pages WHERE fetch_status = 'auth_wall'")?,
            skipped: count("SELECT COUNT(*) FROM pages WHERE fetch_status = 'skip'")?,
            favicons: count("SELECT COUNT(*) FROM favicons")?,
        })
    }

    pub fn has_favicon(&self, host: &str) -> Result<bool> {
        let conn = Connection::open(&self.path)?;
        let n: u32 = conn.query_row(
            "SELECT COUNT(*) FROM favicons WHERE host = ?1",
            [host],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    pub fn store_favicon(&self, host: &str, mime: &str, data: &[u8]) -> Result<()> {
        let conn = Connection::open(&self.path)?;
        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        conn.execute(
            "INSERT INTO favicons(host, mime, data, fetched_at) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(host) DO UPDATE SET mime=excluded.mime, data=excluded.data, fetched_at=excluded.fetched_at",
            params![host, mime, data, now],
        )?;
        Ok(())
    }

    pub fn get_favicon(&self, host: &str) -> Result<Option<(String, Vec<u8>)>> {
        let conn = Connection::open(&self.path)?;
        conn.query_row(
            "SELECT mime, data FROM favicons WHERE host = ?1",
            [host],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn get_bodies(&self, urls: &[String]) -> Result<Vec<(String, String)>> {
        let conn = Connection::open(&self.path)?;
        let mut out = Vec::with_capacity(urls.len());
        for url in urls {
            let body: Option<String> = conn
                .query_row("SELECT body FROM pages WHERE url = ?1", [url], |r| r.get(0))
                .optional()?;
            if let Some(b) = body {
                out.push((url.clone(), b));
            }
        }
        Ok(out)
    }

    pub fn delete_page(&self, url: &str) -> Result<()> {
        let conn = Connection::open(&self.path)?;
        let id: Option<i64> = conn
            .query_row("SELECT id FROM pages WHERE url = ?1", [url], |r| r.get(0))
            .optional()?;
        if let Some(id) = id {
            conn.execute("DELETE FROM pages_fts WHERE rowid = ?1", [id])?;
            conn.execute("DELETE FROM pages WHERE id = ?1", [id])?;
        }
        Ok(())
    }

    pub fn delete_host(&self, host: &str) -> Result<u64> {
        let conn = Connection::open(&self.path)?;
        let ids: Vec<i64> = conn
            .prepare("SELECT id FROM pages WHERE instr(url, ?1) > 0")?
            .query_map([host], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        for id in &ids {
            conn.execute("DELETE FROM pages_fts WHERE rowid = ?1", [id])?;
        }
        conn.execute("DELETE FROM pages WHERE instr(url, ?1) > 0", [host])?;
        Ok(ids.len() as u64)
    }

    pub fn ban_host(&self, host: &str) -> Result<u64> {
        let conn = Connection::open(&self.path)?;
        conn.execute(
            "INSERT OR IGNORE INTO banned_hosts(host) VALUES(?1)",
            [host],
        )?;
        let deleted = self.delete_host(host)?;
        Ok(deleted)
    }

    pub fn get_banned_hosts(&self) -> Result<Vec<String>> {
        let conn = Connection::open(&self.path)?;
        let hosts = conn
            .prepare("SELECT host FROM banned_hosts")?
            .query_map([], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(hosts)
    }

    pub fn add_cluster_ignored_domain(&self, domain: &str) -> Result<()> {
        let conn = Connection::open(&self.path)?;
        conn.execute(
            "INSERT OR IGNORE INTO cluster_ignored_domains(domain) VALUES(?1)",
            [domain],
        )?;
        Ok(())
    }

    pub fn get_cluster_ignored_domains(&self) -> Result<Vec<String>> {
        let conn = Connection::open(&self.path)?;
        let domains = conn
            .prepare("SELECT domain FROM cluster_ignored_domains")?
            .query_map([], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(domains)
    }

    pub fn remove_cluster_ignored_domain(&self, domain: &str) -> Result<()> {
        let conn = Connection::open(&self.path)?;
        conn.execute(
            "DELETE FROM cluster_ignored_domains WHERE domain = ?1",
            [domain],
        )?;
        Ok(())
    }

    pub fn set_starred(&self, url: &str, starred: bool) -> Result<()> {
        let conn = Connection::open(&self.path)?;
        conn.execute(
            "UPDATE pages SET starred = ?1 WHERE url = ?2",
            params![starred as i32, url],
        )?;
        Ok(())
    }

    /// Insert the URL if it doesn't exist, then star it. Used by the browser bookmark button.
    pub fn bookmark(&self, url: &str, title: &str) -> Result<()> {
        let conn = Connection::open(&self.path)?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO pages(url, title, fetch_status, first_visit_at, last_visit_at, starred)
             VALUES(?1, ?2, 'pending', ?3, ?3, 1)
             ON CONFLICT(url) DO UPDATE SET
               title = CASE WHEN excluded.title != '' THEN excluded.title ELSE title END,
               starred = 1,
               fetch_status = 'pending',
               last_visit_at = ?3,
               first_visit_at = COALESCE(first_visit_at, excluded.first_visit_at)",
            params![url, title, now],
        )?;
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        let conn = Connection::open(&self.path)?;
        conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .map_err(Into::into)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        let conn = Connection::open(&self.path)?;
        conn.execute(
            "INSERT INTO meta(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn export_all(&self) -> Result<FullExport> {
        let conn = Connection::open(&self.path)?;
        let pages = conn
            .prepare(
                "SELECT url, title, body, starred, first_visit_at, last_visit_at
                 FROM pages WHERE fetch_status = 'fetched'
                 ORDER BY last_visit_at DESC NULLS LAST",
            )?
            .query_map([], |r| {
                Ok(ExportPage {
                    url: r.get(0)?,
                    title: r.get(1)?,
                    body: r.get(2)?,
                    starred: r.get::<_, i32>(3)? != 0,
                    first_visit_at: r.get(4)?,
                    last_visit_at: r.get(5)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        let ban_list = conn
            .prepare("SELECT host FROM banned_hosts ORDER BY host")?
            .query_map([], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(FullExport {
            version: 1,
            exported_at: chrono::Utc::now().to_rfc3339(),
            pages,
            ban_list,
        })
    }

    pub fn import_all(&self, export: &FullExport) -> Result<(u64, u64)> {
        let mut conn = Connection::open(&self.path)?;
        let tx = conn.transaction()?;
        let mut pages_imported = 0u64;
        for page in &export.pages {
            if !page.url.starts_with("http://") && !page.url.starts_with("https://") {
                continue;
            }
            let existing: Option<i64> = tx
                .query_row("SELECT id FROM pages WHERE url = ?1", [&page.url], |r| {
                    r.get(0)
                })
                .optional()?;
            if let Some(id) = existing {
                tx.execute("DELETE FROM pages_fts WHERE rowid = ?1", [id])?;
                tx.execute(
                    "UPDATE pages SET title=?1, body=?2, fetch_status='fetched',
                     starred=MAX(starred,?3),
                     first_visit_at=COALESCE(first_visit_at,?4),
                     last_visit_at=CASE WHEN ?5 > COALESCE(last_visit_at,'') THEN ?5 ELSE last_visit_at END
                     WHERE id=?6",
                    params![
                        page.title,
                        page.body,
                        page.starred as i32,
                        page.first_visit_at,
                        page.last_visit_at,
                        id
                    ],
                )?;
                tx.execute(
                    "INSERT INTO pages_fts(rowid, title, body) VALUES(?1, ?2, ?3)",
                    params![id, page.title, page.body],
                )?;
            } else {
                tx.execute(
                    "INSERT INTO pages(url,title,body,fetch_status,starred,first_visit_at,last_visit_at) \
                     VALUES(?1,?2,?3,'fetched',?4,?5,?6)",
                    params![
                        page.url,
                        page.title,
                        page.body,
                        page.starred as i32,
                        page.first_visit_at,
                        page.last_visit_at
                    ],
                )?;
                let id = tx.last_insert_rowid();
                if !page.body.is_empty() {
                    tx.execute(
                        "INSERT INTO pages_fts(rowid, title, body) VALUES(?1, ?2, ?3)",
                        params![id, page.title, page.body],
                    )?;
                }
            }
            pages_imported += 1;
        }
        tx.commit()?;
        let conn2 = Connection::open(&self.path)?;
        let mut bans_imported = 0u64;
        for pattern in &export.ban_list {
            if !pattern.is_empty() {
                conn2.execute(
                    "INSERT OR IGNORE INTO banned_hosts(host) VALUES(?1)",
                    [pattern.as_str()],
                )?;
                bans_imported += 1;
            }
        }
        Ok((pages_imported, bans_imported))
    }

    pub fn import_starred(&self, items: &[(String, String)]) -> Result<u64> {
        let mut count = 0u64;
        for (url, title) in items {
            if url.starts_with("http://") || url.starts_with("https://") {
                self.bookmark(url, title)?;
                count += 1;
            }
        }
        Ok(count)
    }

    pub fn get_pages_for_clustering(
        &self,
        days: u32,
    ) -> Result<Vec<crate::cluster::PageForClustering>> {
        let conn = Connection::open(&self.path)?;
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(days as i64))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let mut stmt = conn.prepare(
            "SELECT url, title, last_visit_at, embedding
             FROM pages
             WHERE last_visit_at >= ?1
             ORDER BY last_visit_at ASC",
        )?;
        let rows = stmt
            .query_map([cutoff], |r| {
                let ts: String = r.get(2)?;
                let emb_bytes: Option<Vec<u8>> = r.get(3)?;
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    ts,
                    emb_bytes,
                ))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(url, title, ts, emb_bytes)| {
                let visited_at =
                    chrono::NaiveDateTime::parse_from_str(&ts, "%Y-%m-%dT%H:%M:%S%.f%#z")
                        .or_else(|_| {
                            chrono::NaiveDateTime::parse_from_str(&ts, "%Y-%m-%dT%H:%M:%SZ")
                        })
                        .or_else(|_| {
                            chrono::NaiveDateTime::parse_from_str(&ts, "%Y-%m-%d %H:%M:%S")
                        })
                        .ok()?;
                let embedding = emb_bytes.map(|b| {
                    b.chunks_exact(4)
                        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                        .collect::<Vec<f32>>()
                });
                Some(crate::cluster::PageForClustering {
                    url,
                    title,
                    visited_at,
                    embedding,
                })
            })
            .collect();
        Ok(rows)
    }

    pub fn get_page(&self, url: &str) -> Result<Option<serde_json::Value>> {
        let conn = Connection::open(&self.path)?;
        let row = conn
            .query_row(
                "SELECT url, title, body, fetch_status, starred, first_visit_at, last_visit_at
             FROM pages WHERE url = ?1",
                [url],
                |r| {
                    Ok(serde_json::json!({
                        "url": r.get::<_, String>(0)?,
                        "title": r.get::<_, String>(1)?,
                        "body": r.get::<_, String>(2)?,
                        "fetch_status": r.get::<_, String>(3)?,
                        "starred": r.get::<_, i32>(4)? != 0,
                        "first_visit_at": r.get::<_, Option<String>>(5)?,
                        "last_visit_at": r.get::<_, Option<String>>(6)?,
                    }))
                },
            )
            .optional()?;
        Ok(row)
    }

    pub fn get_starred(&self, limit: u32) -> Result<Vec<PageEntry>> {
        let conn = Connection::open(&self.path)?;
        let rows = conn
            .prepare(
                "SELECT url, title, fetch_status, starred
             FROM pages WHERE starred = 1
             ORDER BY title
             LIMIT ?1",
            )?
            .query_map([limit], |r| {
                Ok(PageEntry {
                    url: r.get(0)?,
                    title: r.get(1)?,
                    fetch_status: r.get(2)?,
                    starred: r.get::<_, i32>(3)? != 0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>();
        Ok(rows)
    }

    pub fn list_pages(
        &self,
        limit: u32,
        offset: u32,
        filter: Option<&str>,
    ) -> Result<Vec<PageEntry>> {
        let conn = Connection::open(&self.path)?;
        let rows: Vec<PageEntry> = if let Some(f) = filter.filter(|s| !s.is_empty()) {
            conn.prepare(
                "SELECT url, title, fetch_status, starred FROM pages
                 WHERE instr(lower(url), lower(?1)) > 0 OR instr(lower(title), lower(?1)) > 0
                 ORDER BY last_visit_at DESC NULLS LAST
                 LIMIT ?2 OFFSET ?3",
            )?
            .query_map(params![f, limit, offset], |r| {
                Ok(PageEntry {
                    url: r.get(0)?,
                    title: r.get(1)?,
                    fetch_status: r.get(2)?,
                    starred: r.get::<_, i32>(3)? != 0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect()
        } else {
            conn.prepare(
                "SELECT url, title, fetch_status, starred FROM pages
                 ORDER BY last_visit_at DESC NULLS LAST
                 LIMIT ?1 OFFSET ?2",
            )?
            .query_map(params![limit, offset], |r| {
                Ok(PageEntry {
                    url: r.get(0)?,
                    title: r.get(1)?,
                    fetch_status: r.get(2)?,
                    starred: r.get::<_, i32>(3)? != 0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect()
        };
        Ok(rows)
    }

    /// Increments fetch_attempts. If attempts reach `max_attempts`, marks the
    /// URL as 'error' so it stops being retried.
    pub fn record_fetch_error(&self, url: &str, max_attempts: u32) -> Result<()> {
        let conn = Connection::open(&self.path)?;
        conn.execute(
            "UPDATE pages SET fetch_attempts = fetch_attempts + 1,
                              fetch_status = CASE WHEN fetch_attempts + 1 >= ?1 THEN 'error' ELSE fetch_status END
             WHERE url = ?2",
            params![max_attempts, url],
        )?;
        Ok(())
    }

    pub fn mark_status(&self, url: &str, status: FetchStatus) -> Result<()> {
        let conn = Connection::open(&self.path)?;
        conn.execute(
            "UPDATE pages SET fetch_status = ?1 WHERE url = ?2",
            params![status, url],
        )?;
        Ok(())
    }

    pub fn urls_needing_fetch(&self, limit: u32) -> Result<Vec<String>> {
        let conn = Connection::open(&self.path)?;
        let mut stmt =
            conn.prepare("SELECT url FROM pages WHERE fetch_status = 'pending' LIMIT ?1")?;
        stmt.query_map([limit], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn search(&self, query: &str, limit: u32) -> Result<Vec<SearchResult>> {
        let conn = Connection::open(&self.path)?;

        // Full-text search over page title + body.
        let mut fts_results: Vec<SearchResult> = conn
            .prepare(
                "SELECT p.url, p.title,
                        snippet(pages_fts, 1, '<b>', '</b>', '...', 20),
                        bm25(pages_fts),
                        p.first_visit_at, p.last_visit_at, p.starred
                 FROM pages_fts
                 JOIN pages p ON p.id = pages_fts.rowid
                 WHERE pages_fts MATCH ?1
                 ORDER BY bm25(pages_fts)
                 LIMIT ?2",
            )?
            .query_map(params![query, limit], |row| {
                Ok(SearchResult {
                    url: row.get(0)?,
                    title: row.get(1)?,
                    snippet: row.get(2)?,
                    rank: row.get(3)?,
                    first_visit_at: row.get(4)?,
                    last_visit_at: row.get(5)?,
                    starred: row.get::<_, i32>(6)? != 0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        // URL substring match — catches pages found via URL slug (e.g. "aerocano"
        // in the path) that may not have the word in indexed body text.
        let seen: std::collections::HashSet<String> =
            fts_results.iter().map(|r| r.url.clone()).collect();
        let remaining = limit.saturating_sub(fts_results.len() as u32);
        if remaining > 0 {
            let url_hits: Vec<SearchResult> = conn
                .prepare(
                    "SELECT url, COALESCE(NULLIF(title,''), url), '', 0.0,
                            first_visit_at, last_visit_at, starred
                     FROM pages
                     WHERE instr(url, ?1) > 0
                     LIMIT ?2",
                )?
                .query_map(params![query, remaining], |row| {
                    Ok(SearchResult {
                        url: row.get(0)?,
                        title: row.get(1)?,
                        snippet: row.get(2)?,
                        rank: row.get(3)?,
                        first_visit_at: row.get(4)?,
                        last_visit_at: row.get(5)?,
                        starred: row.get::<_, i32>(6)? != 0,
                    })
                })?
                .filter_map(|r| r.ok())
                .filter(|r| !seen.contains(&r.url))
                .collect();
            fts_results.extend(url_hits);
        }

        Ok(fts_results)
    }

    pub fn store_embedding(&self, url: &str, embedding: &[f32]) -> Result<()> {
        let conn = Connection::open(&self.path)?;
        conn.execute(
            "UPDATE pages SET embedding = ?1 WHERE url = ?2",
            params![embed_to_bytes(embedding), url],
        )?;
        Ok(())
    }

    /// Pages that have been fetched but not yet embedded.
    pub fn pages_needing_embedding(&self, limit: u32) -> Result<Vec<(String, String, String)>> {
        let conn = Connection::open(&self.path)?;
        let mut stmt = conn.prepare(
            "SELECT url, title, body FROM pages
             WHERE fetch_status = 'fetched' AND embedding IS NULL
             LIMIT ?1",
        )?;
        stmt.query_map([limit], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Cosine-similarity search over all stored embeddings. Only returns results
    /// with score ≥ `min_score` so zero-vectors don't pollute RAG context.
    /// Returns pages visited in the last `days_back` days that are fetched,
    /// plus a map of host → page count for the prior `days_back` days (novelty).
    pub fn weekly_pages(
        &self,
        days_back: i64,
    ) -> Result<(Vec<WeeklyEntry>, std::collections::HashMap<String, usize>)> {
        let conn = Connection::open(&self.path)?;
        let cutoff = format!("-{days_back} days");
        let prior_cutoff = format!("-{} days", days_back * 2);

        let entries: Vec<WeeklyEntry> = conn
            .prepare(
                "SELECT url, title,
                        substr(body, 1, 300) as snippet,
                        last_visit_at
                 FROM pages
                 WHERE fetch_status = 'fetched'
                   AND last_visit_at >= datetime('now', ?1)
                 ORDER BY last_visit_at DESC",
            )?
            .query_map([&cutoff], |r| {
                let url: String = r.get(0)?;
                let host = crate::config::host_from_url(&url).to_string();
                Ok(WeeklyEntry {
                    url,
                    title: r.get(1)?,
                    snippet: r.get(2)?,
                    host,
                    last_visit_at: r.get(3)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        let prior_counts: std::collections::HashMap<String, usize> = {
            let rows: Vec<String> = conn
                .prepare(
                    "SELECT url FROM pages
                     WHERE fetch_status = 'fetched'
                       AND last_visit_at >= datetime('now', ?1)
                       AND last_visit_at < datetime('now', ?2)",
                )?
                .query_map([&prior_cutoff, &cutoff], |r| r.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            let mut map = std::collections::HashMap::new();
            for url in rows {
                let host = crate::config::host_from_url(&url).to_string();
                *map.entry(host).or_insert(0) += 1;
            }
            map
        };

        Ok((entries, prior_counts))
    }

    /// Fetch full SearchResult rows for a set of URLs (no snippet; rank = 0).
    /// Used to enrich vector-only hits that FTS5 didn't return.
    pub fn fetch_by_urls(&self, urls: &[String]) -> Result<Vec<SearchResult>> {
        if urls.is_empty() {
            return Ok(vec![]);
        }
        let conn = Connection::open(&self.path)?;
        let placeholders = urls
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT url, COALESCE(NULLIF(title,''), url), first_visit_at, last_visit_at, starred
             FROM pages WHERE url IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            urls.iter().map(|u| u as &dyn rusqlite::ToSql).collect();
        let rows = stmt
            .query_map(params.as_slice(), |row| {
                Ok(SearchResult {
                    url: row.get(0)?,
                    title: row.get(1)?,
                    snippet: String::new(),
                    rank: 0.0,
                    first_visit_at: row.get(2)?,
                    last_visit_at: row.get(3)?,
                    starred: row.get::<_, i32>(4)? != 0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn vector_search(
        &self,
        query: &[f32],
        limit: u32,
        min_score: f32,
    ) -> Result<Vec<VectorResult>> {
        let conn = Connection::open(&self.path)?;
        let mut stmt =
            conn.prepare("SELECT url, title, embedding FROM pages WHERE embedding IS NOT NULL")?;

        let mut candidates: Vec<(String, String, f32)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            })?
            .filter_map(|r| match r {
                Ok(v) => Some(v),
                Err(e) => {
                    warn!(error = %e, "vector search: skipping row");
                    None
                }
            })
            .filter_map(|(url, title, bytes)| {
                let emb = bytes_to_vec(&bytes);
                if emb.len() == query.len() {
                    Some((url, title, cosine_similarity(query, &emb)))
                } else {
                    None
                }
            })
            .filter(|(_, _, score)| *score >= min_score)
            .collect();

        candidates.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

        Ok(candidates
            .into_iter()
            .take(limit as usize)
            .map(|(url, title, score)| VectorResult { url, title, score })
            .collect())
    }
}

fn embed_to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn bytes_to_vec(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_store() -> (IndexStore, TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = IndexStore::open(&dir.path().join("index.db")).unwrap();
        (store, dir)
    }

    #[test]
    fn upsert_and_search() {
        let (store, _dir) = test_store();
        store
            .upsert_page(
                "https://example.com",
                "Example Page",
                "This page is about Rust programming language features.",
            )
            .unwrap();
        let results = store.search("rust", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://example.com");
    }

    #[test]
    fn search_returns_empty_for_no_match() {
        let (store, _dir) = test_store();
        store
            .upsert_page("https://example.com", "Hello", "Hello world content")
            .unwrap();
        let results = store.search("zzznomatch999", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn upsert_updates_existing_page() {
        let (store, _dir) = test_store();
        store
            .upsert_page(
                "https://example.com",
                "Old Title",
                "old content about nothing",
            )
            .unwrap();
        store
            .upsert_page(
                "https://example.com",
                "New Title",
                "new content about Rust async",
            )
            .unwrap();
        let old = store.search("nothing", 10).unwrap();
        assert!(old.is_empty(), "old content should be gone from index");
        let new = store.search("async", 10).unwrap();
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].title, "New Title");
    }

    #[test]
    fn urls_needing_fetch_returns_pending() {
        let (store, _dir) = test_store();
        store
            .register_urls(
                ["https://a.com", "https://b.com", "https://c.com"]
                    .iter()
                    .copied(),
            )
            .unwrap();
        store
            .mark_status("https://b.com", FetchStatus::AuthWall)
            .unwrap();
        let pending = store.urls_needing_fetch(10).unwrap();
        assert_eq!(pending.len(), 2);
        assert!(!pending.contains(&"https://b.com".to_string()));
    }

    #[test]
    fn cosine_similarity_basic() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);

        let c = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, &c).abs() < 1e-6);

        // Zero vector → 0
        let z = vec![0.0, 0.0, 0.0];
        assert_eq!(cosine_similarity(&a, &z), 0.0);
    }

    #[test]
    fn embed_roundtrip() {
        let orig = vec![1.0f32, -0.5, 3.14];
        let bytes = embed_to_bytes(&orig);
        let back = bytes_to_vec(&bytes);
        for (a, b) in orig.iter().zip(&back) {
            assert!((a - b).abs() < 1e-7);
        }
    }

    #[test]
    fn vector_search_returns_best_match() {
        let (store, _dir) = test_store();
        store
            .upsert_page("https://rust.org", "Rust", "systems programming")
            .unwrap();
        store
            .upsert_page("https://python.org", "Python", "dynamic scripting")
            .unwrap();

        let rust_vec = vec![1.0f32, 0.0, 0.0, 0.0];
        let python_vec = vec![0.0f32, 1.0, 0.0, 0.0];
        store
            .store_embedding("https://rust.org", &rust_vec)
            .unwrap();
        store
            .store_embedding("https://python.org", &python_vec)
            .unwrap();

        let query = vec![1.0f32, 0.0, 0.0, 0.0]; // identical to rust_vec
        let results = store.vector_search(&query, 5, 0.0).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "https://rust.org");
        assert!((results[0].score - 1.0).abs() < 1e-6);
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn vector_search_min_score_filters_low_similarity() {
        let (store, _dir) = test_store();
        store
            .upsert_page("https://a.com", "A", "content a")
            .unwrap();
        store
            .upsert_page("https://b.com", "B", "content b")
            .unwrap();

        let a_vec = vec![1.0f32, 0.0];
        let b_vec = vec![0.0f32, 1.0];
        store.store_embedding("https://a.com", &a_vec).unwrap();
        store.store_embedding("https://b.com", &b_vec).unwrap();

        let query = vec![1.0f32, 0.0]; // orthogonal to b_vec
        let results = store.vector_search(&query, 5, 0.5).unwrap();
        assert_eq!(
            results.len(),
            1,
            "only a.com should clear the 0.5 threshold"
        );
        assert_eq!(results[0].url, "https://a.com");
    }

    #[test]
    fn pages_needing_embedding_returns_fetched_without_embedding() {
        let (store, _dir) = test_store();
        store
            .upsert_page("https://fetched.com", "F", "content")
            .unwrap();
        store
            .register_urls(["https://pending.com"].iter().copied())
            .unwrap();

        let pending = store.pages_needing_embedding(10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, "https://fetched.com");

        store
            .store_embedding("https://fetched.com", &[1.0, 0.0])
            .unwrap();
        let pending2 = store.pages_needing_embedding(10).unwrap();
        assert!(pending2.is_empty());
    }
}
