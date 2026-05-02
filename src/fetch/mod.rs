pub mod extract;

use std::time::Duration;

use reqwest::Client;
use tracing::{debug, warn};

use crate::config::FetchSettings;
use extract::{extract, is_auth_wall, ExtractedPage};

#[derive(Debug)]
pub enum FetchResult {
    Ok(ExtractedPage),
    AuthWall,
    Skip,
    Error(String),
}

pub struct Fetcher {
    client: Client,
    delay: Duration,
}

impl Fetcher {
    pub fn new(settings: &FetchSettings) -> anyhow::Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(settings.timeout_secs))
            .user_agent(&settings.user_agent)
            .build()?;
        Ok(Self {
            client,
            delay: Duration::from_millis(settings.delay_ms),
        })
    }

    /// Fetches `/favicon.ico` from the same origin as `page_url`.
    /// Returns `(host, mime, bytes)` on success, `None` otherwise.
    pub async fn fetch_favicon(&self, page_url: &str) -> Option<(String, String, Vec<u8>)> {
        let (scheme, rest) = if let Some(r) = page_url.strip_prefix("https://") {
            ("https", r)
        } else if let Some(r) = page_url.strip_prefix("http://") {
            ("http", r)
        } else {
            return None;
        };
        let authority_end = rest.find('/').unwrap_or(rest.len());
        let authority = &rest[..authority_end];
        let host_key = match authority.rsplit_once(':') {
            Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => h,
            _ => authority,
        };
        let favicon_url = format!("{scheme}://{authority}/favicon.ico");
        let resp = self.client.get(&favicon_url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let mime = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/x-icon")
            .to_string();
        let bytes = resp.bytes().await.ok()?;
        if bytes.is_empty() {
            return None;
        }
        Some((host_key.to_string(), mime, bytes.to_vec()))
    }

    pub async fn fetch(&self, url: &str) -> FetchResult {
        tokio::time::sleep(self.delay).await;
        let resp = match self.client.get(url).send().await {
            Ok(r) => r,
            Err(e) => {
                warn!(%url, error = %e, "fetch failed");
                return FetchResult::Error(e.to_string());
            }
        };
        let final_url = resp.url().to_string();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();
        if !content_type.contains("text/html") {
            return FetchResult::Skip;
        }
        let html = match resp.text().await {
            Ok(t) => t,
            Err(e) => return FetchResult::Error(e.to_string()),
        };
        if !is_auth_wall(&final_url, &html) {
            return FetchResult::Ok(extract(&html));
        }
        // Auth wall — try the Wayback Machine for a cached copy.
        if let Some(page) = self.fetch_wayback(url).await {
            return FetchResult::Ok(page);
        }
        FetchResult::AuthWall
    }

    async fn fetch_wayback(&self, url: &str) -> Option<ExtractedPage> {
        debug!(%url, "querying Wayback CDX for most recent snapshot");
        // limit=-1 means "last N results" in CDX — returns the most recent snapshot.
        let rows: Vec<Vec<String>> = match self
            .client
            .get("https://web.archive.org/cdx/search/cdx")
            .query(&[
                ("url", url),
                ("output", "json"),
                ("limit", "-1"),
                ("filter", "statuscode:200"),
                ("fl", "timestamp"),
            ])
            .send()
            .await
        {
            Ok(r) => r.json().await.unwrap_or_default(),
            Err(e) => { warn!(%url, error = %e, "wayback CDX request failed"); return None; }
        };
        // rows[0] is the header ["timestamp"], rows[1] is the data row.
        let timestamp = match rows.get(1).and_then(|r| r.first()) {
            Some(t) => t.clone(),
            None => { debug!(%url, "no Wayback snapshot found in CDX"); return None; }
        };
        let snapshot_url = format!("https://web.archive.org/web/{}/{}", timestamp, url);
        debug!(%url, snapshot = %snapshot_url, "fetching Wayback snapshot");
        let snap_resp = match self.client.get(&snapshot_url).send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => { warn!(%url, status = %r.status(), "wayback snapshot returned error"); return None; }
            Err(e) => { warn!(%url, error = %e, "wayback snapshot fetch failed"); return None; }
        };
        let html = match snap_resp.text().await {
            Ok(h) => h,
            Err(e) => { warn!(%url, error = %e, "wayback snapshot body read failed"); return None; }
        };
        let page = extract(&html);
        if page.body.is_empty() {
            warn!(%url, "wayback snapshot had empty body after extraction");
            return None;
        }
        debug!(%url, "wayback snapshot extracted successfully");
        Some(page)
    }
}
