use std::path::Path;

use chrono::NaiveDateTime;
use rusqlite::{Connection, Result as SqlResult};

use super::{BrowserHistory, HistoryItem};

pub struct OrionBrowser;

impl BrowserHistory for OrionBrowser {
    fn recent(&self, conn: &Connection, limit: u32) -> anyhow::Result<Vec<HistoryItem>> {
        recent(conn, limit).map_err(Into::into)
    }

    fn top_sites(&self, conn: &Connection, limit: u32) -> anyhow::Result<Vec<HistoryItem>> {
        top_sites(conn, limit).map_err(Into::into)
    }

    fn reading_list_items(&self, history_db_path: &Path) -> Vec<(String, String)> {
        let plist_path = history_db_path
            .parent()
            .map(|p| p.join("ReadingList.plist"))
            .unwrap_or_default();
        read_reading_list(&plist_path)
    }
}

fn read_reading_list(path: &Path) -> Vec<(String, String)> {
    let value = match plist::Value::from_file(path) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    collect_items(&value, &mut out);
    out
}

fn collect_items(value: &plist::Value, out: &mut Vec<(String, String)>) {
    match value {
        plist::Value::Array(arr) => {
            for item in arr {
                collect_items(item, out);
            }
        }
        plist::Value::Dictionary(dict) => {
            let url = dict
                .get("URLString")
                .or_else(|| dict.get("url"))
                .or_else(|| dict.get("URL"))
                .and_then(|v| v.as_string())
                .map(str::to_owned);

            if let Some(url) = url {
                if url.starts_with("http://") || url.starts_with("https://") {
                    let title = dict
                        .get("title")
                        .and_then(|v| v.as_string())
                        .or_else(|| {
                            dict.get("URIDictionary")
                                .and_then(|v| v.as_dictionary())
                                .and_then(|d| d.get("title"))
                                .and_then(|v| v.as_string())
                        })
                        .unwrap_or("")
                        .to_owned();
                    out.push((url, title));
                }
            } else {
                for (_, v) in dict.iter() {
                    collect_items(v, out);
                }
            }
        }
        _ => {}
    }
}

pub fn open(path: &std::path::Path) -> SqlResult<Connection> {
    Connection::open(path)
}

pub fn recent(conn: &Connection, limit: u32) -> SqlResult<Vec<HistoryItem>> {
    let mut stmt = conn.prepare(
        "SELECT ID, URL, TITLE, HOST, LAST_VISIT_TIME, VISIT_COUNT
         FROM history_items
         ORDER BY LAST_VISIT_TIME DESC
         LIMIT ?1",
    )?;
    stmt.query_map([limit], map_row)?.collect()
}

pub fn top_sites(conn: &Connection, limit: u32) -> SqlResult<Vec<HistoryItem>> {
    let mut stmt = conn.prepare(
        "SELECT ID, URL, TITLE, HOST, LAST_VISIT_TIME, VISIT_COUNT
         FROM history_items
         ORDER BY VISIT_COUNT DESC
         LIMIT ?1",
    )?;
    stmt.query_map([limit], map_row)?.collect()
}

fn map_row(row: &rusqlite::Row<'_>) -> SqlResult<HistoryItem> {
    Ok(HistoryItem {
        id: row.get(0)?,
        url: row.get(1)?,
        title: row.get(2)?,
        host: row.get(3)?,
        last_visit_time: parse_timestamp(row.get(4)?),
        visit_count: row.get(5)?,
    })
}

// Orion stores timestamps as "YYYY-MM-DD HH:MM:SS"; chrono's default SQLite
// format uses a T separator, so we parse manually.
fn parse_timestamp(s: String) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S"))
        .unwrap_or(chrono::DateTime::UNIX_EPOCH.naive_utc())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn in_memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE history_items (
                ID INTEGER PRIMARY KEY AUTOINCREMENT,
                URL TEXT, TITLE TEXT, HOST TEXT,
                LAST_VISIT_TIME TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                VISIT_COUNT INTEGER, TYPED_COUNT INTEGER
            );
            INSERT INTO history_items (URL, TITLE, HOST, LAST_VISIT_TIME, VISIT_COUNT) VALUES
                ('https://a.com', 'A', 'a.com', '2026-04-30 10:00:00', 3),
                ('https://b.com', 'B', 'b.com', '2026-04-30 09:00:00', 9),
                ('https://c.com', 'C', 'c.com', '2026-04-29 08:00:00', 1);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn recent_returns_items_newest_first() {
        let conn = in_memory_db();
        let items = recent(&conn, 10).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].url, "https://a.com");
        assert_eq!(items[2].url, "https://c.com");
    }

    #[test]
    fn recent_respects_limit() {
        let conn = in_memory_db();
        let items = recent(&conn, 2).unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn top_sites_returns_highest_visit_count_first() {
        let conn = in_memory_db();
        let items = top_sites(&conn, 10).unwrap();
        assert_eq!(items[0].url, "https://b.com");
        assert_eq!(items[0].visit_count, 9);
    }

    #[test]
    fn parse_timestamp_handles_space_separator() {
        let dt = parse_timestamp("2026-04-30 10:00:00".to_string());
        assert_eq!(dt.to_string(), "2026-04-30 10:00:00");
    }

    #[test]
    fn orion_browser_trait_recent() {
        let conn = in_memory_db();
        let items = OrionBrowser.recent(&conn, 10).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].url, "https://a.com");
    }
}
