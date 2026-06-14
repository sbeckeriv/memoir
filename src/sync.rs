use std::sync::Arc;

use thiserror::Error;
use tracing::{debug, info, warn};

use crate::browser;
use crate::config::{self, Settings};
use crate::embed::EmbedText;
use crate::fetch::recipe::{looks_like_recipe, parse_llm_recipe, recipe_extract_prompt, url_host};
use crate::fetch::{FetchResult, Fetcher};
use crate::index::store::IndexError;
use crate::index::{FetchStatus, IndexStore};
use crate::rag::LlmClient;
use crate::session_log::{LogKind, SessionLog};

const BROWSER_HISTORY_FETCH_LIMIT: u32 = 1000;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("browser history unavailable: {0}")]
    BrowserAccess(#[from] std::io::Error),

    #[error("index operation failed: {0}")]
    Index(#[from] IndexError),

    #[error("fetch failed: {0}")]
    Fetch(String),

    #[error("embedding failed: {0}")]
    Embedding(String),

    #[error("task join error: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),

    #[error("operation failed: {0}")]
    Other(#[from] anyhow::Error),
}

fn slog(
    log: &Option<Arc<SessionLog>>,
    kind: LogKind,
    msg: impl Into<String>,
    detail: impl Into<Option<String>>,
) {
    if let Some(l) = log {
        l.push(kind, msg, detail);
    }
}

pub async fn run(
    config: &Settings,
    embedder: Option<Arc<dyn EmbedText>>,
    log: Option<Arc<SessionLog>>,
) -> Result<(), SyncError> {
    info!(
        data_dir   = %config.data.dir.display(),
        history_db = %config.browser.history_db_path.display(),
        delay_ms   = config.fetch.delay_ms,
        timeout_s  = config.fetch.timeout_secs,
        ban        = ?config.fetch.ban,
        llm_model  = %config.llm.model,
        "config loaded"
    );
    slog(&log, LogKind::Sync, "Sync started", None);

    let index_path = config.data.dir.join("index.db");
    info!(db = %index_path.display(), "opening index");
    let index = IndexStore::open(&index_path)?;

    let fetcher = Fetcher::new(&config.fetch)?;

    let current_mtime = std::fs::metadata(&config.browser.history_db_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs().to_string());

    let stored_mtime = {
        let index = index.clone();
        tokio::task::spawn_blocking(move || index.get_meta("browser_db_mtime")).await??
    };

    let db_changed = current_mtime.as_deref() != stored_mtime.as_deref();

    let urls = if db_changed {
        info!(history_db = %config.browser.history_db_path.display(), "browser DB changed, syncing visits");
        slog(
            &log,
            LogKind::Sync,
            "Browser history changed — reading new visits",
            None,
        );
        let snapshot = browser::copy_db(&config.browser.history_db_path)?;
        debug!(tmp = %snapshot.path().display(), "browser DB snapshot ready");

        let b = browser::for_config(&config.browser);
        let fetch_settings = config.fetch.clone();
        let orig_db_path = config.browser.history_db_path.clone();
        let fetch_batch = config.sync.fetch_batch;
        let urls = {
            let index = index.clone();
            let snap_path = snapshot.path().to_path_buf();
            tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<String>> {
                let conn = rusqlite::Connection::open(&snap_path)?;
                let items = b.recent(&conn, BROWSER_HISTORY_FETCH_LIMIT)?;
                debug!("browser history has {} items", items.len());
                let visits: Vec<(String, String)> = items
                    .iter()
                    .filter(|i| !fetch_settings.is_banned(&i.url))
                    .map(|i| {
                        let ts = i.last_visit_time.format("%Y-%m-%d %H:%M:%S").to_string();
                        (i.url.clone(), ts)
                    })
                    .collect();
                debug!("registering {} visits (after ban filter)", visits.len());
                index.register_visits(visits.iter().map(|(u, t)| (u.as_str(), t.as_str())))?;

                let rl = b.reading_list_items(&orig_db_path);
                if !rl.is_empty() {
                    debug!("registering {} reading list URLs", rl.len());
                    index.register_urls(rl.iter().map(|(u, _)| u.as_str()))?;
                }

                Ok(index.urls_needing_fetch(fetch_batch)?)
            })
            .await??
        };

        if let Some(mtime) = current_mtime {
            let index = index.clone();
            tokio::task::spawn_blocking(move || index.set_meta("browser_db_mtime", &mtime))
                .await??;
        }

        drop(snapshot);
        urls
    } else {
        debug!("browser DB unchanged, skipping visit registration");
        slog(
            &log,
            LogKind::Sync,
            "Browser history unchanged — checking for pending pages",
            None,
        );
        let index = index.clone();
        let fetch_batch = config.sync.fetch_batch;
        tokio::task::spawn_blocking(move || index.urls_needing_fetch(fetch_batch)).await??
    };

    // Build set of hosts to skip for recipe detection (many pages, zero recipes).
    let recipe_skip = match tokio::task::spawn_blocking({
        let idx = index.clone();
        move || idx.recipe_skip_hosts(15)
    })
    .await
    {
        Ok(Ok(skip)) => skip,
        Ok(Err(e)) => {
            warn!(error = %e, "failed to build recipe skip list");
            Default::default()
        }
        Err(e) => {
            warn!(error = %e, "recipe skip task panicked");
            Default::default()
        }
    };

    info!("syncing {} URLs", urls.len());
    if urls.is_empty() {
        slog(&log, LogKind::Sync, "No pages to fetch", None);
    } else {
        slog(
            &log,
            LogKind::Sync,
            format!("Fetching {} page(s)", urls.len()),
            None,
        );
    }

    for url in urls {
        if config.fetch.is_banned(&url) {
            let index = index.clone();
            let url2 = url.clone();
            tokio::task::spawn_blocking(move || index.mark_status(&url2, FetchStatus::Skip))
                .await??;
            debug!(%url, "skipping banned URL");
            continue;
        }
        debug!(%url, "fetching");
        match fetcher.fetch(&url).await {
            FetchResult::Ok(page) => {
                let recipe = page.recipe.clone();
                let title = page.title.clone();
                let body = page.body.clone();
                let idx = index.clone();
                let url2 = url.clone();
                tokio::task::spawn_blocking(move || {
                    idx.upsert_page(&url2, &page.title, &page.body)
                })
                .await??;

                // Recipe handling
                let host_skip = url_host(&url)
                    .map(|h| recipe_skip.contains(h))
                    .unwrap_or(false);
                if !host_skip {
                    if let Some(card) = recipe {
                        // Schema.org recipe found — store immediately.
                        let idx = index.clone();
                        let url2 = url.clone();
                        if let Err(e) = tokio::task::spawn_blocking(move || {
                            idx.store_recipe(&url2, &card, "schema")
                        })
                        .await
                        {
                            warn!(%url, error = %e, "failed to store schema recipe");
                        } else {
                            info!(%url, "recipe stored from schema");
                        }
                    } else if looks_like_recipe(&title, &body) {
                        // Heuristic match — queue for LLM extraction.
                        let idx = index.clone();
                        let url2 = url.clone();
                        if let Err(e) = tokio::task::spawn_blocking(move || {
                            idx.set_recipe_status(&url2, "pending")
                        })
                        .await
                        {
                            warn!(%url, error = %e, "failed to set recipe_status=pending");
                        }
                    } else {
                        let idx = index.clone();
                        let url2 = url.clone();
                        if let Err(e) = tokio::task::spawn_blocking(move || {
                            idx.set_recipe_status(&url2, "none")
                        })
                        .await
                        {
                            warn!(%url, error = %e, "failed to set recipe_status=none");
                        }
                    }
                }

                // Fetch and store favicon if we don't have one for this host yet.
                let host = config::host_from_url(&url).to_string();
                let needs = {
                    let idx = index.clone();
                    let host2 = host.clone();
                    tokio::task::spawn_blocking(move || idx.has_favicon(&host2).map(|has| !has))
                        .await??
                };
                if needs && let Some((favicon_host, mime, data)) = fetcher.fetch_favicon(&url).await
                {
                    let idx = index.clone();
                    tokio::task::spawn_blocking(move || {
                        idx.store_favicon(&favicon_host, &mime, &data)
                    })
                    .await??;
                }

                info!(%url, "indexed");
                slog(&log, LogKind::Sync, format!("Indexed: {url}"), None);
            }
            FetchResult::AuthWall => {
                let index = index.clone();
                let url2 = url.clone();
                tokio::task::spawn_blocking(move || {
                    index.mark_status(&url2, FetchStatus::AuthWall)
                })
                .await??;
                warn!(%url, "auth wall, skipping");
                slog(
                    &log,
                    LogKind::Sync,
                    format!("Skipped (auth wall): {url}"),
                    None,
                );
            }
            FetchResult::Paywall => {
                let index = index.clone();
                let url2 = url.clone();
                tokio::task::spawn_blocking(move || index.mark_status(&url2, FetchStatus::Paywall))
                    .await??;
                warn!(%url, "paywall, skipping");
                slog(
                    &log,
                    LogKind::Sync,
                    format!("Skipped (paywall): {url}"),
                    None,
                );
            }
            FetchResult::Skip => {
                let index = index.clone();
                let url2 = url.clone();
                tokio::task::spawn_blocking(move || index.mark_status(&url2, FetchStatus::Skip))
                    .await??;
            }
            FetchResult::Error(e) => {
                let index = index.clone();
                let url2 = url.clone();
                let max_retries = config.fetch.max_retries;
                tokio::task::spawn_blocking(move || index.record_fetch_error(&url2, max_retries))
                    .await??;
                warn!(%url, error = %e, "fetch error, will retry next sync (up to 3 attempts)");
                slog(
                    &log,
                    LogKind::Error,
                    format!("Fetch error: {url}"),
                    Some(e.to_string()),
                );
            }
        }
    }

    if let Some(embedder) = embedder {
        let to_embed = {
            let index = index.clone();
            let embed_batch = config.sync.embed_batch;
            tokio::task::spawn_blocking(move || index.pages_needing_embedding(embed_batch))
                .await??
        };
        info!("embedding {} pages", to_embed.len());
        if !to_embed.is_empty() {
            slog(
                &log,
                LogKind::Sync,
                format!("Embedding {} page(s)", to_embed.len()),
                None,
            );
        }

        let mut embed_ok = 0usize;
        for (url, title, body) in to_embed {
            let text = format!("{title} {body}");
            let emb_result = {
                let embedder = embedder.clone();
                tokio::task::spawn_blocking(move || embedder.embed_one(&text)).await
            };
            match emb_result {
                Ok(Ok(vec)) => {
                    let index = index.clone();
                    let url2 = url.clone();
                    tokio::task::spawn_blocking(move || index.store_embedding(&url2, &vec))
                        .await??;
                    info!(%url, "embedded");
                    embed_ok += 1;
                }
                Ok(Err(e)) => {
                    warn!(%url, error = %e, "embedding failed");
                    slog(
                        &log,
                        LogKind::Error,
                        format!("Embed error: {url}"),
                        Some(e.to_string()),
                    );
                }
                Err(e) => {
                    warn!(%url, error = %e, "embedding task panicked");
                    slog(
                        &log,
                        LogKind::Error,
                        format!("Embed panic: {url}"),
                        Some(e.to_string()),
                    );
                }
            }
        }
        if embed_ok > 0 {
            slog(
                &log,
                LogKind::Sync,
                format!("Embedding complete — {embed_ok} page(s) embedded"),
                None,
            );
        }
    } else {
        slog(
            &log,
            LogKind::Sync,
            "Semantic indexing skipped (no embedding model)",
            None,
        );
    }

    // LLM recipe extraction pass — process up to 20 pending pages per cycle.
    let pending_recipes = match tokio::task::spawn_blocking({
        let idx = index.clone();
        move || idx.pages_needing_recipe_llm(20)
    })
    .await
    {
        Ok(Ok(pages)) => pages,
        Ok(Err(e)) => {
            warn!(error = %e, "failed to query pending recipe pages");
            vec![]
        }
        Err(e) => {
            warn!(error = %e, "recipe query task panicked");
            vec![]
        }
    };
    if !pending_recipes.is_empty() {
        slog(
            &log,
            LogKind::Sync,
            format!(
                "Extracting recipes from {} page(s) via LLM",
                pending_recipes.len()
            ),
            None,
        );
        let llm = LlmClient::new(&config.llm);
        for (url, title, body) in pending_recipes {
            let prompt = recipe_extract_prompt(&title, &body);
            match llm.generate(&prompt, None).await {
                Ok(response) => {
                    if let Some(card) = parse_llm_recipe(&response) {
                        let recipe_name = card.name.clone();
                        let idx = index.clone();
                        let url2 = url.clone();
                        match tokio::task::spawn_blocking(move || {
                            idx.store_recipe(&url2, &card, "llm")
                        })
                        .await
                        {
                            Ok(Ok(())) => {
                                info!(%url, "recipe extracted via LLM");
                                slog(
                                    &log,
                                    LogKind::Sync,
                                    format!("Recipe found: {recipe_name}"),
                                    Some(url.clone()),
                                );
                            }
                            Ok(Err(e)) => warn!(%url, error = %e, "failed to store LLM recipe"),
                            Err(e) => warn!(%url, error = %e, "store_recipe task panicked"),
                        }
                    } else {
                        let idx = index.clone();
                        let url2 = url.clone();
                        match tokio::task::spawn_blocking(move || {
                            idx.set_recipe_status(&url2, "none")
                        })
                        .await
                        {
                            Ok(Ok(())) => {
                                slog(
                                    &log,
                                    LogKind::Sync,
                                    "Recipe check: no recipe found".to_string(),
                                    Some(url.clone()),
                                );
                            }
                            Ok(Err(e)) => {
                                warn!(%url, error = %e, "failed to set recipe_status=none")
                            }
                            Err(e) => warn!(%url, error = %e, "set_recipe_status task panicked"),
                        }
                    }
                }
                Err(e) => {
                    warn!(%url, error = %e, "LLM recipe extraction failed");
                    slog(
                        &log,
                        LogKind::Error,
                        format!("Recipe extraction error: {url}"),
                        Some(e.to_string()),
                    );
                }
            }
        }
    }

    slog(&log, LogKind::Sync, "Sync complete", None);
    Ok(())
}
