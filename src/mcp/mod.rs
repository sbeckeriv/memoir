use std::io::{BufRead, Write};

use serde_json::{json, Value};

use crate::index::IndexStore;

pub fn run(index: IndexStore) -> anyhow::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Notifications have no "id" — don't respond.
        let id = match msg.get("id") {
            Some(id) => id.clone(),
            None => continue,
        };

        let method = msg["method"].as_str().unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(json!({}));

        let result = match method {
            "initialize" => handle_initialize(),
            "tools/list" => handle_tools_list(),
            "tools/call" => handle_tools_call(&index, &params),
            _ => Err(format!("unknown method: {method}")),
        };

        let response = match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32600, "message": e }
            }),
        };

        writeln!(out, "{}", serde_json::to_string(&response)?)?;
        out.flush()?;
    }

    Ok(())
}

fn handle_initialize() -> Result<Value, String> {
    Ok(json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "memoir", "version": "0.1" }
    }))
}

fn handle_tools_list() -> Result<Value, String> {
    Ok(json!({
        "tools": [
            {
                "name": "search",
                "description": "Full-text search across your indexed browsing history.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search terms" },
                        "limit": { "type": "integer", "description": "Max results (default 10)" }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "get_page",
                "description": "Retrieve the stored content of a specific page by URL.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "Exact URL of the page" }
                    },
                    "required": ["url"]
                }
            },
            {
                "name": "get_recent",
                "description": "List recently visited pages from the index, newest first.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit": { "type": "integer", "description": "Max results (default 20)" }
                    }
                }
            },
            {
                "name": "get_starred",
                "description": "List pages you have starred / bookmarked.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit": { "type": "integer", "description": "Max results (default 50)" }
                    }
                }
            }
        ]
    }))
}

fn handle_tools_call(index: &IndexStore, params: &Value) -> Result<Value, String> {
    let name = params["name"].as_str().ok_or("missing tool name")?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    let content = match name {
        "search" => {
            let query = args["query"].as_str().ok_or("missing query")?;
            let limit = args["limit"].as_u64().unwrap_or(10) as u32;
            let results = index.search(query, limit).map_err(|e| e.to_string())?;
            let text = if results.is_empty() {
                "No results found.".to_string()
            } else {
                results
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        format!(
                            "{}. {}\n   URL: {}\n   {}",
                            i + 1,
                            if r.title.is_empty() { &r.url } else { &r.title },
                            r.url,
                            r.snippet
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n")
            };
            json!([{ "type": "text", "text": text }])
        }
        "get_page" => {
            let url = args["url"].as_str().ok_or("missing url")?;
            let page = index.get_page(url).map_err(|e| e.to_string())?;
            let text = match page {
                None => format!("No page found for URL: {url}"),
                Some(p) => {
                    let title = p["title"].as_str().unwrap_or("");
                    let body = p["body"].as_str().unwrap_or("");
                    let starred = p["starred"].as_bool().unwrap_or(false);
                    let visited = p["last_visit_at"].as_str().unwrap_or("unknown");
                    let preview: String = body.chars().take(4000).collect();
                    format!("Title: {title}\nURL: {url}\nStarred: {starred}\nLast visited: {visited}\n\n{preview}")
                }
            };
            json!([{ "type": "text", "text": text }])
        }
        "get_recent" => {
            let limit = args["limit"].as_u64().unwrap_or(20) as u32;
            let pages = index.list_pages(limit, 0, None).map_err(|e| e.to_string())?;
            let text = if pages.is_empty() {
                "No pages in index.".to_string()
            } else {
                pages
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let title = if p.title.is_empty() { p.url.as_str() } else { p.title.as_str() };
                        format!("{}. {}\n   {}", i + 1, title, p.url)
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n")
            };
            json!([{ "type": "text", "text": text }])
        }
        "get_starred" => {
            let limit = args["limit"].as_u64().unwrap_or(50) as u32;
            let pages = index.get_starred(limit).map_err(|e| e.to_string())?;
            let text = if pages.is_empty() {
                "No starred pages.".to_string()
            } else {
                pages
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let title = if p.title.is_empty() { p.url.as_str() } else { p.title.as_str() };
                        format!("{}. {}\n   {}", i + 1, title, p.url)
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n")
            };
            json!([{ "type": "text", "text": text }])
        }
        other => return Err(format!("unknown tool: {other}")),
    };

    Ok(json!({ "content": content }))
}
