use std::collections::VecDeque;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::Serialize;

const MAX_SYNC: usize = 500;
const MAX_LLM: usize = 100;
const MAX_SEARCH: usize = 500;
const MAX_ERROR: usize = 200;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LogKind {
    Sync,
    Llm,
    Search,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub ts: DateTime<Utc>,
    pub kind: LogKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

struct Inner {
    sync: VecDeque<LogEntry>,
    llm: VecDeque<LogEntry>,
    search: VecDeque<LogEntry>,
    error: VecDeque<LogEntry>,
}

pub struct SessionLog {
    inner: Mutex<Inner>,
}

impl SessionLog {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                sync: VecDeque::new(),
                llm: VecDeque::new(),
                search: VecDeque::new(),
                error: VecDeque::new(),
            }),
        }
    }

    pub fn push(
        &self,
        kind: LogKind,
        message: impl Into<String>,
        detail: impl Into<Option<String>>,
    ) {
        let entry = LogEntry {
            ts: Utc::now(),
            kind: kind.clone(),
            message: message.into(),
            detail: detail.into(),
        };
        let mut inner = self.inner.lock().unwrap();
        let (deque, max) = match kind {
            LogKind::Sync => (&mut inner.sync, MAX_SYNC),
            LogKind::Llm => (&mut inner.llm, MAX_LLM),
            LogKind::Search => (&mut inner.search, MAX_SEARCH),
            LogKind::Error => (&mut inner.error, MAX_ERROR),
        };
        deque.push_front(entry);
        if deque.len() > max {
            deque.pop_back();
        }
    }

    pub fn get_all(&self) -> Vec<LogEntry> {
        let inner = self.inner.lock().unwrap();
        let mut all: Vec<LogEntry> = inner
            .sync
            .iter()
            .chain(inner.llm.iter())
            .chain(inner.search.iter())
            .chain(inner.error.iter())
            .cloned()
            .collect();
        all.sort_by_key(|b| std::cmp::Reverse(b.ts));
        all
    }

    pub fn get_by_kind(&self, kind: &str) -> Vec<LogEntry> {
        let inner = self.inner.lock().unwrap();
        match kind {
            "sync" => inner.sync.iter().cloned().collect(),
            "llm" => inner.llm.iter().cloned().collect(),
            "search" => inner.search.iter().cloned().collect(),
            "error" => inner.error.iter().cloned().collect(),
            _ => vec![],
        }
    }
}

impl Default for SessionLog {
    fn default() -> Self {
        Self::new()
    }
}
