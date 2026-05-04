mod handlers;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use axum::http::{Method, header};
use axum::{
    Router,
    routing::{delete, get, post},
};
use tower_http::cors::{Any, CorsLayer};

use crate::browser::BrowserHistory;
use crate::config::Settings;
use crate::embed::EmbedText;
use crate::index::IndexStore;
use crate::rag::LlmClient;

#[derive(Clone)]
pub struct AppState {
    pub browser_db_path: PathBuf,
    pub browser: Arc<dyn BrowserHistory>,
    pub index: IndexStore,
    pub embedder: Option<Arc<dyn EmbedText>>,
    pub llm: Arc<LlmClient>,
    pub config: Arc<RwLock<Settings>>,
    pub sync_paused: Arc<AtomicBool>,
    pub palette_hide: Arc<tokio::sync::Notify>,
    pub update_requested: Arc<tokio::sync::Notify>,
    pub update_status: Arc<tokio::sync::Mutex<String>>,
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
        let llm = Arc::new(LlmClient::new(&config.llm));
        llm.ensure_loaded().await;
        let browser = crate::browser::for_config(&config.browser);
        let palette_hide = Arc::new(tokio::sync::Notify::new());
        let update_requested = Arc::new(tokio::sync::Notify::new());
        let update_status = Arc::new(tokio::sync::Mutex::new(String::new()));
        let log = Arc::new(crate::session_log::SessionLog::new());
        let state = AppState {
            browser_db_path: config.browser.history_db_path.clone(),
            browser,
            index,
            embedder,
            llm,
            config: Arc::new(RwLock::new(config.clone())),
            sync_paused: sync_paused.clone(),
            palette_hide: palette_hide.clone(),
            update_requested: update_requested.clone(),
            update_status: update_status.clone(),
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
        .route("/api/ask", get(handlers::ask_get).post(handlers::ask))
        .route("/api/stats", get(handlers::stats))
        .route("/api/favicon", get(handlers::favicon))
        .route("/api/pages", get(handlers::list_pages))
        .route("/api/starred", get(handlers::starred))
        .route("/api/star", post(handlers::set_starred))
        .route("/api/page", delete(handlers::delete_page))
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
        .route("/mcp", post(handlers::mcp_http))
        .layer(cors)
        .with_state(state)
}
