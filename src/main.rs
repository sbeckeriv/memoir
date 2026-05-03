use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use memoir::{Application, Embedder, EmbedText, Settings};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("memoir=debug")),
        )
        .with_writer(std::io::stderr)
        .init();

    let raw: Vec<String> = std::env::args().skip(1).collect();
    let (config_dir, subcommand, no_sync) = parse_args(&raw);

    let config = match config_dir {
        Some(dir) => Settings::load_from(&dir),
        None => Settings::load(),
    };

    let embedder: Option<Arc<dyn EmbedText>> =
        tokio::task::spawn_blocking(|| Embedder::try_new().ok())
            .await
            .ok()
            .flatten()
            .map(|e| Arc::new(e) as Arc<dyn EmbedText>);

    match subcommand {
        Some("sync") => memoir::sync::run(&config, embedder, None).await?,
        _ => {
            let sync_paused = Arc::new(AtomicBool::new(false));
            let app = Application::build(config.clone(), embedder.clone(), sync_paused.clone()).await?;
            eprintln!("Listening on http://127.0.0.1:{}", app.port());

            tokio::spawn(memoir::mcp::run(app.state.clone()));

            if !no_sync {
                let cfg = config.clone();
                let emb = embedder.clone();
                let log = app.log.clone();
                tokio::spawn(async move {
                    if let Err(e) = memoir::sync::run(&cfg, emb, Some(log)).await {
                        tracing::warn!(error = %e, "startup sync failed");
                    }
                });

                let sp = sync_paused.clone();
                let log = app.log.clone();
                tokio::spawn(async move {
                    loop {
                        let cfg = Settings::load();
                        let secs = cfg.sync.interval_mins.max(1) * 60;
                        tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                        if sp.load(std::sync::atomic::Ordering::Relaxed) {
                            continue;
                        }
                        let embedder: Option<Arc<dyn EmbedText>> =
                            tokio::task::spawn_blocking(|| Embedder::try_new().ok())
                                .await
                                .ok()
                                .flatten()
                                .map(|e| Arc::new(e) as Arc<dyn EmbedText>);
                        if let Err(e) = memoir::sync::run(&cfg, embedder, Some(log.clone())).await {
                            tracing::warn!(error = %e, "scheduled sync failed");
                        }
                    }
                });
            }

            app.run_until_stopped().await?;
        }
    }

    Ok(())
}

fn parse_args(args: &[String]) -> (Option<PathBuf>, Option<&str>, bool) {
    let mut config_dir: Option<PathBuf> = None;
    let mut subcommand: Option<&str> = None;
    let mut no_sync = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--config-dir" => {
                if let Some(val) = args.get(i + 1) {
                    config_dir = Some(PathBuf::from(val));
                    i += 2;
                } else {
                    eprintln!("memoir: --config-dir requires a path");
                    i += 1;
                }
            }
            "--no-sync" => {
                no_sync = true;
                i += 1;
            }
            s if subcommand.is_none() => {
                subcommand = Some(s);
                i += 1;
            }
            _ => { i += 1; }
        }
    }
    (config_dir, subcommand, no_sync)
}
