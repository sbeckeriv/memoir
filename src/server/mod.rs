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
    pub log: Arc<crate::session_log::SessionLog>,
}

pub struct Application {
    port: u16,
    listener: tokio::net::TcpListener,
    router: Router,
    sync_paused: Arc<AtomicBool>,
    palette_hide: Arc<tokio::sync::Notify>,
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
        .route("/api/top-sites", get(handlers::top_sites))
        .route("/api/search", get(handlers::search))
        .route("/api/ask", post(handlers::ask))
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
        .route("/api/export/starred", get(handlers::export_starred))
        .route("/api/import/starred", post(handlers::import_starred))
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
        .route("/mcp", post(handlers::mcp_http))
        .layer(cors)
        .with_state(state)
}
