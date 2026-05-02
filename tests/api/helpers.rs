use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use memoir::{
    Application, ApplicationSettings, BrowserKind, BrowserSettings, DataSettings, EmbedText,
    FetchSettings, IndexStore, LlmSettings, Settings,
};
use rusqlite::Connection;
use std::path::PathBuf;
use tempfile::TempDir;

struct ZeroEmbedder;

impl EmbedText for ZeroEmbedder {
    fn embed_one(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
        Ok(vec![0.0; 384])
    }
}

pub struct TestApp {
    pub address: String,
    pub client: reqwest::Client,
    _db_dir: TempDir,
    _data_dir: TempDir,
}

async fn start_mock_ollama() -> String {
    let router = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(|| async {
            axum::Json(serde_json::json!({
                "choices": [{"message": {"role": "assistant", "content": "Mock answer from LLM."}, "finish_reason": "stop"}]
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind mock ollama listener");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://127.0.0.1:{port}")
}

pub async fn spawn_app() -> TestApp {
    let ollama_url = start_mock_ollama().await;
    let (db_path, _db_dir) = create_test_db();
    let _data_dir = tempfile::tempdir().expect("failed to create data dir");
    let data_dir = _data_dir.path().to_path_buf();

    let index_path = data_dir.join("index.db");
    let index = IndexStore::open(&index_path).expect("failed to open index");
    seed_index(&index);

    let embedder: Option<Arc<dyn EmbedText>> = Some(Arc::new(ZeroEmbedder));

    let config = Settings {
        application: ApplicationSettings {
            host: "127.0.0.1".to_string(),
            port: 0,
            ..ApplicationSettings::default()
        },
        browser: BrowserSettings {
            history_db_path: db_path,
            kind: BrowserKind::Orion,
        },
        data: DataSettings { dir: data_dir },
        fetch: FetchSettings::default(),
        llm: LlmSettings {
            base_url: ollama_url,
            model: "test-model".to_string(),
            ..LlmSettings::default()
        },
        ..Settings::default()
    };

    let sync_paused = Arc::new(AtomicBool::new(false));
    let app = Application::build(config, embedder, sync_paused).await.expect("failed to build app");
    let port = app.port();
    tokio::spawn(app.run_until_stopped());

    TestApp {
        address: format!("http://127.0.0.1:{port}"),
        client: reqwest::Client::new(),
        _db_dir,
        _data_dir,
    }
}

fn create_test_db() -> (PathBuf, TempDir) {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let db_path = dir.path().join("history");

    let conn = Connection::open(&db_path).expect("failed to open test db");
    conn.execute_batch(
        "CREATE TABLE history_items (
            ID INTEGER PRIMARY KEY AUTOINCREMENT,
            URL TEXT, TITLE TEXT, HOST TEXT,
            LAST_VISIT_TIME TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            VISIT_COUNT INTEGER, TYPED_COUNT INTEGER
        );
        INSERT INTO history_items (URL, TITLE, HOST, LAST_VISIT_TIME, VISIT_COUNT) VALUES
            ('https://example.com/page1', 'Page One',  'example.com',  '2026-04-30 10:00:00', 5),
            ('https://rust-lang.org',     'Rust',       'rust-lang.org','2026-04-30 09:30:00', 12),
            ('https://crates.io',         'crates.io',  'crates.io',    '2026-04-30 09:00:00', 8),
            ('https://docs.rs',           'Docs.rs',    'docs.rs',      '2026-04-29 22:00:00', 20),
            ('https://github.com',        'GitHub',     'github.com',   '2026-04-29 18:00:00', 50);",
    )
    .expect("failed to seed test data");

    (db_path, dir)
}

fn seed_index(index: &IndexStore) {
    index
        .upsert_page(
            "https://rust-lang.org",
            "The Rust Programming Language",
            "Rust is a systems programming language focused on safety, speed, and concurrency.",
        )
        .unwrap();
    index
        .upsert_page(
            "https://tokio.rs",
            "Tokio - Async Runtime for Rust",
            "Tokio is an asynchronous runtime for writing reliable network applications in Rust.",
        )
        .unwrap();
    index
        .upsert_page(
            "https://docs.rs/serde",
            "Serde - Serialization Framework",
            "Serde is a framework for serializing and deserializing Rust data structures efficiently.",
        )
        .unwrap();
}
