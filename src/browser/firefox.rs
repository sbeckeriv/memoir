use chrono::NaiveDateTime;
use rusqlite::Connection;

use super::{BrowserHistory, HistoryItem};

pub struct FirefoxBrowser;

fn firefox_timestamp_to_naive(micros: i64) -> NaiveDateTime {
    let secs = micros / 1_000_000;
    let nsecs = ((micros % 1_000_000).unsigned_abs() as u32) * 1000;
    chrono::DateTime::from_timestamp(secs, nsecs)
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
        last_visit_time: firefox_timestamp_to_naive(row.get::<_, i64>(3).unwrap_or(0)),
        visit_count: row.get(4)?,
    })
}

impl BrowserHistory for FirefoxBrowser {
    fn recent(&self, conn: &Connection, limit: u32) -> anyhow::Result<Vec<HistoryItem>> {
        let mut stmt = conn.prepare(
            "SELECT id, url, title, last_visit_date, visit_count
             FROM moz_places
             WHERE last_visit_date IS NOT NULL AND visit_count > 0
             ORDER BY last_visit_date DESC
             LIMIT ?1",
        )?;
        Ok(stmt
            .query_map([limit], map_row)?
            .collect::<rusqlite::Result<_>>()?)
    }

    fn top_sites(&self, conn: &Connection, limit: u32) -> anyhow::Result<Vec<HistoryItem>> {
        let mut stmt = conn.prepare(
            "SELECT id, url, title, last_visit_date, visit_count
             FROM moz_places
             WHERE visit_count > 0
             ORDER BY visit_count DESC
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
    use rusqlite::Connection;

    // Firefox stores microseconds since Unix epoch.
    // Unix 1777399200 = 2026-04-28 18:00:00 UTC → Firefox micros 1777399200000000
    fn in_memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE moz_places (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                url TEXT, title TEXT,
                last_visit_date INTEGER,
                visit_count INTEGER DEFAULT 0
            );
            INSERT INTO moz_places (url, title, last_visit_date, visit_count) VALUES
                ('https://a.com/page', 'A', 1777399200000000, 3),
                ('https://b.com/page', 'B', 1777397400000000, 9),
                ('https://c.com/page', 'C', 1777395600000000, 1);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn recent_returns_items_newest_first() {
        let conn = in_memory_db();
        let items = FirefoxBrowser.recent(&conn, 10).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].url, "https://a.com/page");
        assert_eq!(items[2].url, "https://c.com/page");
    }

    #[test]
    fn recent_respects_limit() {
        let conn = in_memory_db();
        let items = FirefoxBrowser.recent(&conn, 2).unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn top_sites_returns_highest_visit_count_first() {
        let conn = in_memory_db();
        let items = FirefoxBrowser.top_sites(&conn, 10).unwrap();
        assert_eq!(items[0].url, "https://b.com/page");
        assert_eq!(items[0].visit_count, 9);
    }

    #[test]
    fn host_is_extracted_from_url() {
        let conn = in_memory_db();
        let items = FirefoxBrowser.recent(&conn, 1).unwrap();
        assert_eq!(items[0].host, "a.com");
    }

    #[test]
    fn timestamp_conversion_is_correct() {
        let conn = in_memory_db();
        let items = FirefoxBrowser.recent(&conn, 1).unwrap();
        assert_eq!(items[0].last_visit_time.to_string(), "2026-04-28 18:00:00");
    }
}
