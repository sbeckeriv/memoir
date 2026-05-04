use chrono::NaiveDateTime;
use rusqlite::Connection;

use super::{BrowserHistory, HistoryItem};

pub struct SafariBrowser;

// Safari stores visit_time as seconds since 2001-01-01 (Core Data reference date).
// Unix epoch offset: 2001-01-01 00:00:00 UTC = 978307200 seconds after 1970-01-01.
const CORE_DATA_OFFSET: i64 = 978_307_200;

fn safari_time(secs: f64) -> NaiveDateTime {
    let unix = secs as i64 + CORE_DATA_OFFSET;
    chrono::DateTime::from_timestamp(unix, 0)
        .map(|dt| dt.naive_utc())
        .unwrap_or(chrono::DateTime::UNIX_EPOCH.naive_utc())
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryItem> {
    let url: String = row.get(1)?;
    let host = crate::config::host_from_url(&url).to_string();
    Ok(HistoryItem {
        id: row.get(0)?,
        url,
        title: row.get(2)?,
        host,
        last_visit_time: safari_time(row.get::<_, f64>(3).unwrap_or(0.0)),
        visit_count: row.get(4)?,
    })
}

impl BrowserHistory for SafariBrowser {
    fn recent(&self, conn: &Connection, limit: u32) -> anyhow::Result<Vec<HistoryItem>> {
        let mut stmt = conn.prepare(
            "SELECT hi.id, hi.url, hv.title, hv.visit_time, hi.visit_count
             FROM history_visits hv
             JOIN history_items hi ON hv.history_item = hi.id
             ORDER BY hv.visit_time DESC
             LIMIT ?1",
        )?;
        Ok(stmt
            .query_map([limit], map_row)?
            .collect::<rusqlite::Result<_>>()?)
    }

    fn top_sites(&self, conn: &Connection, limit: u32) -> anyhow::Result<Vec<HistoryItem>> {
        let mut stmt = conn.prepare(
            "SELECT hi.id, hi.url,
                    (SELECT hv.title FROM history_visits hv
                     WHERE hv.history_item = hi.id ORDER BY hv.visit_time DESC LIMIT 1),
                    (SELECT MAX(hv.visit_time) FROM history_visits hv
                     WHERE hv.history_item = hi.id),
                    hi.visit_count
             FROM history_items hi
             WHERE hi.visit_count > 0
             ORDER BY hi.visit_count DESC
             LIMIT ?1",
        )?;
        Ok(stmt
            .query_map([limit], map_row)?
            .collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE history_items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                url TEXT NOT NULL UNIQUE,
                visit_count INTEGER DEFAULT 0
            );
            CREATE TABLE history_visits (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                history_item INTEGER NOT NULL REFERENCES history_items(id),
                visit_time REAL,
                title TEXT
            );
            INSERT INTO history_items (url, visit_count) VALUES
                ('https://a.com/page', 3),
                ('https://b.com/page', 9);
            -- visit_time 0.0 → 2001-01-01 00:00:00 UTC
            INSERT INTO history_visits (history_item, visit_time, title) VALUES
                (1, 100.0, 'Page A'),
                (2,  50.0, 'Page B');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn recent_returns_newest_first() {
        let conn = in_memory_db();
        let items = SafariBrowser.recent(&conn, 10).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].url, "https://a.com/page");
    }

    #[test]
    fn top_sites_returns_highest_visit_count_first() {
        let conn = in_memory_db();
        let items = SafariBrowser.top_sites(&conn, 10).unwrap();
        assert_eq!(items[0].url, "https://b.com/page");
        assert_eq!(items[0].visit_count, 9);
    }

    #[test]
    fn timestamp_epoch_offset_is_applied() {
        let conn = in_memory_db();
        let items = SafariBrowser.recent(&conn, 1).unwrap();
        // visit_time 100.0 → unix 978307300 → 2001-01-01 00:01:40 UTC
        assert_eq!(items[0].last_visit_time.to_string(), "2001-01-01 00:01:40");
    }
}
