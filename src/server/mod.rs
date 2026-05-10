mod handlers;

use std::sync::atomic::{AtomicBool, AtomicI64};
use std::sync::{Arc, RwLock};

use axum::http::{Method, header};
use axum::{
    Router,
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};

use crate::config::Settings;
use crate::embed::EmbedText;
use crate::index::IndexStore;
use crate::rag::LlmClient;

#[derive(Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub version: String,
    pub url: String,
}

fn is_newer_version(latest: &str, current: &str) -> bool {
    let parse = |v: &str| -> Option<(u32, u32, u32)> {
        let v = v.trim_start_matches('v');
        let mut p = v.splitn(3, '.');
        Some((
            p.next()?.parse().ok()?,
            p.next()?.parse().ok()?,
            p.next()?.parse().ok()?,
        ))
    };
    matches!((parse(latest), parse(current)), (Some(l), Some(c)) if l > c)
}

pub async fn check_latest_release(current_ver: &str) -> Option<UpdateInfo> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://api.github.com/repos/sbeckeriv/memoir/releases/latest")
        .header("User-Agent", format!("memoir/{current_ver}"))
        .header("Accept", "application/vnd.github.v3+json")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    let tag_name = json.get("tag_name")?.as_str()?;
    let html_url = json.get("html_url")?.as_str()?;
    if is_newer_version(tag_name, current_ver) {
        Some(UpdateInfo {
            version: tag_name.trim_start_matches('v').to_string(),
            url: html_url.to_string(),
        })
    } else {
        None
    }
}

#[derive(Clone)]
pub struct AppState {
    // Core components
    pub index: IndexStore,
    pub embedder: Option<Arc<dyn EmbedText>>,
    pub llm: Arc<std::sync::Mutex<Arc<LlmClient>>>,
    pub config: Arc<RwLock<Settings>>,

    // Sync state
    pub sync_paused: Arc<AtomicBool>,
    pub last_sync_at: Arc<AtomicI64>,

    // UI state
    pub palette_hide: Arc<tokio::sync::Notify>,
    pub restart_requested: Arc<tokio::sync::Notify>,

    // Update state
    pub update_requested: Arc<tokio::sync::Notify>,
    pub update_status: Arc<tokio::sync::Mutex<String>>,
    pub update_available: Arc<tokio::sync::Mutex<Option<UpdateInfo>>>,

    // Other
    pub embed_status: Arc<tokio::sync::Mutex<String>>,
    pub log: Arc<crate::session_log::SessionLog>,
}

pub struct Application {
    port: u16,
    listener: tokio::net::TcpListener,
    router: Router,
    sync_paused: Arc<AtomicBool>,
    palette_hide: Arc<tokio::sync::Notify>,
    update_requested: Arc<tokio::sync::Notify>,
    update_status: Arc<tokio::sync::Mutex<String>>,
    pub update_available: Arc<tokio::sync::Mutex<Option<UpdateInfo>>>,
    restart_requested: Arc<tokio::sync::Notify>,
    embed_status: Arc<tokio::sync::Mutex<String>>,
    pub log: Arc<crate::session_log::SessionLog>,
    pub state: AppState,
}

impl Application {
    pub async fn build(
        config: Settings,
        embedder: Option<Arc<dyn EmbedText>>,
        sync_paused: Arc<AtomicBool>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let index_path = config.data.dir.join("index.db");
        let index = IndexStore::open(&index_path)?;
        let llm_client = Arc::new(LlmClient::new(&config.llm));
        llm_client.ensure_loaded().await;
        let llm = Arc::new(std::sync::Mutex::new(llm_client));
        let palette_hide = Arc::new(tokio::sync::Notify::new());
        let update_requested = Arc::new(tokio::sync::Notify::new());
        let update_status = Arc::new(tokio::sync::Mutex::new(String::new()));
        let update_available = Arc::new(tokio::sync::Mutex::new(None::<UpdateInfo>));
        let restart_requested = Arc::new(tokio::sync::Notify::new());
        let embed_status = Arc::new(tokio::sync::Mutex::new(if embedder.is_some() {
            "ready".to_string()
        } else {
            String::new()
        }));
        let log = Arc::new(crate::session_log::SessionLog::new());
        let last_sync_at = Arc::new(AtomicI64::new(0));
        let state = AppState {
            index,
            embedder,
            llm,
            config: Arc::new(RwLock::new(config.clone())),
            sync_paused: sync_paused.clone(),
            last_sync_at: last_sync_at.clone(),
            palette_hide: palette_hide.clone(),
            restart_requested: restart_requested.clone(),
            update_requested: update_requested.clone(),
            update_status: update_status.clone(),
            update_available: update_available.clone(),
            embed_status: embed_status.clone(),
            log: log.clone(),
        };
        let router = build_router(state.clone());
        let addr = format!("{}:{}", config.application.host, config.application.port);
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let port = listener.local_addr()?.port();
        Ok(Self {
            port,
            listener,
            router,
            sync_paused,
            palette_hide,
            update_requested,
            update_status,
            update_available,
            restart_requested,
            embed_status,
            log,
            state,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn sync_paused(&self) -> Arc<AtomicBool> {
        self.sync_paused.clone()
    }

    pub fn palette_hide(&self) -> Arc<tokio::sync::Notify> {
        self.palette_hide.clone()
    }

    pub fn update_requested(&self) -> Arc<tokio::sync::Notify> {
        self.update_requested.clone()
    }

    pub fn update_status(&self) -> Arc<tokio::sync::Mutex<String>> {
        self.update_status.clone()
    }

    pub fn restart_requested(&self) -> Arc<tokio::sync::Notify> {
        self.restart_requested.clone()
    }

    pub fn embed_status(&self) -> Arc<tokio::sync::Mutex<String>> {
        self.embed_status.clone()
    }

    pub async fn run_until_stopped(self) -> std::io::Result<()> {
        axum::serve(self.listener, self.router).await
    }
}

fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE]);

    Router::new()
        .route("/", get(handlers::index_page))
        .route("/manage", get(handlers::manage_page))
        .route("/health", get(handlers::health_check))
        .route("/api/recent", get(handlers::recent))
        .route("/api/weekly", get(handlers::weekly))
        .route("/api/top-sites", get(handlers::top_sites))
        .route("/api/search", get(handlers::search))
        .route("/api/autocomplete", get(handlers::autocomplete))
        .route("/api/ask", get(handlers::ask_get).post(handlers::ask))
        .route("/api/stats", get(handlers::stats))
        .route("/api/favicon", get(handlers::favicon))
        .route("/api/pages", get(handlers::list_pages))
        .route("/api/starred", get(handlers::starred))
        .route("/api/star", post(handlers::set_starred))
        .route(
            "/api/page",
            get(handlers::page_body).delete(handlers::delete_page),
        )
        .route("/api/host", delete(handlers::delete_host))
        .route("/api/ban", post(handlers::ban_host))
        .route("/api/bookmark", post(handlers::bookmark))
        .route("/api/clusters", get(handlers::clusters))
        .route(
            "/api/clusters/ignore",
            post(handlers::ignore_cluster_domain).delete(handlers::unignore_cluster_domain),
        )
        .route("/api/sync", post(handlers::trigger_sync))
        .route("/api/sync/status", get(handlers::sync_status))
        .route("/api/sync/pause", post(handlers::sync_pause))
        .route("/setup", get(handlers::setup_page))
        .route("/api/setup/detect", get(handlers::setup_detect))
        .route("/api/setup/test-llm", get(handlers::setup_test_llm))
        .route("/api/setup", post(handlers::setup_save))
        .route("/api/reindex", post(handlers::reindex_page))
        .route("/api/export/starred", get(handlers::export_starred))
        .route("/api/export/markdown", get(handlers::export_markdown))
        .route("/api/export/all", get(handlers::export_all))
        .route("/api/import/starred", post(handlers::import_starred))
        .route("/api/import/all", post(handlers::import_all))
        .route("/api/topic-clusters", get(handlers::topic_clusters))
        .route("/palette", get(handlers::palette_page))
        .route("/api/palette/hide", post(handlers::hide_palette))
        .route("/api/open-url", get(handlers::open_url))
        .route("/settings", get(handlers::settings_page))
        .route(
            "/api/settings",
            get(handlers::get_settings).post(handlers::save_settings),
        )
        .route("/api/custom-css", get(handlers::custom_css))
        .route("/log", get(handlers::log_page))
        .route("/api/log", get(handlers::log_entries))
        .route("/api/version", get(handlers::version))
        .route("/api/update/check", post(handlers::update_check))
        .route("/api/update/status", get(handlers::update_status))
        .route("/api/update/available", get(handlers::update_available))
        .route("/api/update/restart", post(handlers::update_restart))
        .route("/api/embed/status", get(handlers::embed_status))
        .route("/mcp", post(handlers::mcp_http))
        .layer(cors)
        .with_state(state)
}
