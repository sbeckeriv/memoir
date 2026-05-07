use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use clap::{Parser, Subcommand};
use memoir::{Application, EmbedText, Embedder, Settings, config::LlmProvider};

#[derive(Parser)]
#[command(name = "memoir")]
#[command(about = "Personal browser history indexer", long_about = None)]
#[command(version)]
struct Cli {
    /// Path to config directory
    #[arg(long, value_name = "DIR")]
    config_dir: Option<PathBuf>,

    /// Disable background sync
    #[arg(long)]
    no_sync: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a one-shot sync without starting the server
    Sync,
}

async fn load_embedder(config: &Settings) -> Option<Arc<dyn EmbedText>> {
    if !config.embed.enabled || config.llm.provider == LlmProvider::Disabled {
        return None;
    }
    let cache = Settings::config_dir().join("models");
    let model = config.embed.model;
    match tokio::task::spawn_blocking(move || Embedder::try_new(cache, model)).await {
        Ok(Ok(e)) => Some(Arc::new(e) as Arc<dyn EmbedText>),
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "embedding model unavailable");
            None
        }
        Err(err) => {
            tracing::warn!(error = %err, "embedder task panicked");
            None
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("memoir=debug")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    let config = match cli.config_dir {
        Some(dir) => Settings::load_from(&dir),
        None => Settings::load(),
    };

    let embedder = load_embedder(&config).await;

    match cli.command {
        Some(Commands::Sync) => memoir::sync::run(&config, embedder, None).await?,
        None => {
            let sync_paused = Arc::new(AtomicBool::new(false));
            let app =
                Application::build(config.clone(), embedder.clone(), sync_paused.clone()).await?;
            eprintln!("Listening on http://127.0.0.1:{}", app.port());

            tokio::spawn(memoir::mcp::run(app.state.clone()));

            {
                let update_available = app.update_available.clone();
                tokio::spawn(async move {
                    if let Some(info) =
                        memoir::check_latest_release(env!("CARGO_PKG_VERSION")).await
                    {
                        *update_available.lock().await = Some(info);
                    }
                });
            }

            if !cli.no_sync {
                let cfg = config.clone();
                let emb = embedder.clone();
                let log = app.log.clone();
                let last_sync_at = app.state.last_sync_at.clone();
                tokio::spawn(async move {
                    if let Err(e) = memoir::sync::run(&cfg, emb, Some(log)).await {
                        tracing::warn!(error = %e, "startup sync failed");
                    } else {
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0);
                        last_sync_at.store(ts, std::sync::atomic::Ordering::Relaxed);
                    }
                });

                let sp = sync_paused.clone();
                let log = app.log.clone();
                let last_sync_at = app.state.last_sync_at.clone();
                tokio::spawn(async move {
                    loop {
                        let cfg = Settings::load();
                        let secs = cfg.sync.interval_mins.max(1) * 60;
                        tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                        if sp.load(std::sync::atomic::Ordering::Relaxed) {
                            continue;
                        }
                        let embedder = load_embedder(&cfg).await;
                        if let Err(e) = memoir::sync::run(&cfg, embedder, Some(log.clone())).await {
                            tracing::warn!(error = %e, "scheduled sync failed");
                        } else {
                            let ts = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs() as i64)
                                .unwrap_or(0);
                            last_sync_at.store(ts, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                });
            }

            app.run_until_stopped().await?;
        }
    }

    Ok(())
}
