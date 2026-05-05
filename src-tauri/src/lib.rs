use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri_plugin_autostart::MacosLauncher;

use tauri::{
    Manager, WebviewUrl, WebviewWindowBuilder,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
};

// Injected into every page: intercepts external link clicks and routes them
// through the local server's /api/open-url so the OS opens them in the
// default browser instead of navigating the webview.
const INIT_SCRIPT: &str = r#"(function(){
    document.addEventListener('click', function(e) {
        var a = e.target.closest('a[href]');
        if (!a) return;
        var href = a.href;
        if (!href) return;
        if (href.indexOf('127.0.0.1') !== -1) return;
        if (href.startsWith('http://') || href.startsWith('https://')) {
            e.preventDefault();
            fetch('/api/open-url?url=' + encodeURIComponent(href)).catch(function(){});
        }
    }, true);
})();"#;

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("memoir=info")),
        )
        .init();

    let config_exists = memoir::Settings::config_dir().join("config.toml").exists();

    let sync_paused = Arc::new(AtomicBool::new(false));
    let sp_server = sync_paused.clone();
    let sp_loop = sync_paused.clone();
    let sp_tray = sync_paused.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    use tauri_plugin_global_shortcut::ShortcutState;
                    if event.state() == ShortcutState::Pressed
                        && let Some(palette) = app.get_webview_window("palette")
                    {
                        if palette.is_visible().unwrap_or(false) {
                            palette.hide().ok();
                        } else {
                            palette.show().ok();
                            palette.set_focus().ok();
                            let _ = palette.eval("clearPalette()");
                        }
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Second launch: just focus the existing window.
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Hide the window on close rather than destroying it so the app stays
        // alive in the menu bar.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                window.hide().ok();
                api.prevent_close();
            }
        })
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let ah_for_server = app_handle.clone();

            let hotkey = memoir::Settings::load().application.hotkey;

            // Channel to receive the bound port from the async server task.
            let (port_tx, port_rx) = std::sync::mpsc::channel::<u16>();
            let (log_tx, log_rx) = std::sync::mpsc::channel::<Arc<memoir::SessionLog>>();

            // Start Axum server without waiting for embedder (fast startup).
            // Sync + embedding runs in a separate background task afterward.
            tauri::async_runtime::spawn(async move {
                let config = memoir::Settings::load();

                match memoir::Application::build(config.clone(), None, sp_server).await {
                    Ok(server) => {
                        let port = server.port();
                        port_tx.send(port).ok();

                        let log = server.log.clone();
                        log_tx.send(log.clone()).ok();

                        // Startup update check — stores result for the web UI banner.
                        let update_available = server.update_available.clone();
                        tauri::async_runtime::spawn(async move {
                            let current_ver = env!("CARGO_PKG_VERSION");
                            if let Some(info) = memoir::check_latest_release(current_ver).await {
                                *update_available.lock().await = Some(info);
                            }
                        });

                        // Drive palette hide signals from Axum to the Tauri window.
                        let palette_notify = server.palette_hide();
                        let ah_for_palette = ah_for_server.clone();
                        tauri::async_runtime::spawn(async move {
                            loop {
                                palette_notify.notified().await;
                                if let Some(w) = ah_for_palette.get_webview_window("palette") {
                                    w.hide().ok();
                                }
                            }
                        });

                        // Handle update-check requests from the web UI.
                        let update_requested = server.update_requested();
                        let update_status = server.update_status();
                        let update_log = server.log.clone();
                        tauri::async_runtime::spawn(async move {
                            use tauri_plugin_updater::UpdaterExt;
                            loop {
                                update_requested.notified().await;
                                update_log.push(
                                    memoir::LogKind::Sync,
                                    "Update check started",
                                    None,
                                );
                                match ah_for_server.updater() {
                                    Ok(updater) => match updater.check().await {
                                        Ok(Some(update)) => {
                                            let ver = update.version.clone();
                                            update_log.push(
                                                memoir::LogKind::Sync,
                                                format!("Update available: v{ver}"),
                                                None,
                                            );
                                            *update_status.lock().await =
                                                format!("installing:{ver}");
                                            match update
                                                .download_and_install(|_, _| {}, || {})
                                                .await
                                            {
                                                Ok(()) => {
                                                    update_log.push(
                                                        memoir::LogKind::Sync,
                                                        "Update installed, restarting",
                                                        None,
                                                    );
                                                    ah_for_server.restart();
                                                }
                                                Err(e) => {
                                                    update_log.push(
                                                        memoir::LogKind::Error,
                                                        "Update install failed",
                                                        e.to_string(),
                                                    );
                                                    *update_status.lock().await =
                                                        format!("error:{e}");
                                                }
                                            }
                                        }
                                        Ok(None) => {
                                            update_log.push(
                                                memoir::LogKind::Sync,
                                                "Already up to date",
                                                None,
                                            );
                                            *update_status.lock().await = "up_to_date".to_string();
                                        }
                                        Err(e) => {
                                            update_log.push(
                                                memoir::LogKind::Error,
                                                "Update check failed",
                                                e.to_string(),
                                            );
                                            *update_status.lock().await = format!("error:{e}");
                                        }
                                    },
                                    Err(e) => {
                                        update_log.push(
                                            memoir::LogKind::Error,
                                            "Updater unavailable",
                                            e.to_string(),
                                        );
                                        *update_status.lock().await = format!("error:{e}");
                                    }
                                }
                            }
                        });

                        // Kick off the initial sync with embedder in background.
                        let log_init = log.clone();
                        let embed_status = server.embed_status();
                        tauri::async_runtime::spawn(async move {
                            let embedder = if config.embed.enabled {
                                *embed_status.lock().await = "downloading".to_string();
                                log_init.push(
                                    memoir::LogKind::Sync,
                                    "Downloading embedding model",
                                    None,
                                );
                                let cache = memoir::Settings::config_dir().join("models");
                                let model = config.embed.model;
                                match tokio::task::spawn_blocking(move || {
                                    memoir::Embedder::try_new(cache, model)
                                })
                                .await
                                {
                                    Ok(Ok(e)) => {
                                        *embed_status.lock().await = "ready".to_string();
                                        log_init.push(
                                            memoir::LogKind::Sync,
                                            "Embedding model ready",
                                            None,
                                        );
                                        Some(Arc::new(e) as Arc<dyn memoir::EmbedText>)
                                    }
                                    Ok(Err(err)) => {
                                        let msg = err.to_string();
                                        tracing::warn!(error = %msg, "embedding model unavailable");
                                        log_init.push(
                                            memoir::LogKind::Error,
                                            "Embedding model unavailable",
                                            msg.clone(),
                                        );
                                        *embed_status.lock().await = format!("error: {msg}");
                                        None
                                    }
                                    Err(_) => {
                                        log_init.push(
                                            memoir::LogKind::Error,
                                            "Embedding model unavailable",
                                            "task panicked".to_string(),
                                        );
                                        *embed_status.lock().await =
                                            "error: task panicked".to_string();
                                        None
                                    }
                                }
                            } else {
                                *embed_status.lock().await = "disabled".to_string();
                                None
                            };

                            if let Err(e) =
                                memoir::sync::run(&config, embedder, Some(log_init)).await
                            {
                                tracing::warn!(error = %e, "initial sync failed");
                            }
                        });

                        if let Err(e) = server.run_until_stopped().await {
                            tracing::error!(error = %e, "memoir server stopped");
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "failed to start memoir server");
                        port_tx.send(8734).ok();
                    }
                }
            });

            // Wait up to 15 s for the server to bind.
            let port = port_rx
                .recv_timeout(std::time::Duration::from_secs(15))
                .unwrap_or(8734);
            let log = log_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap_or_else(|_| Arc::new(memoir::SessionLog::new()));

            let startup_path = if config_exists { "/" } else { "/setup" };
            let url: url::Url = format!("http://127.0.0.1:{port}{startup_path}")
                .parse()
                .expect("valid startup URL");

            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("Memoir")
                .inner_size(1280.0, 820.0)
                .min_inner_size(800.0, 600.0)
                .initialization_script(INIT_SCRIPT)
                .build()?;

            // Palette window: compact search overlay, hidden until shortcut fires.
            let palette_url: url::Url = format!("http://127.0.0.1:{port}/palette")
                .parse()
                .expect("valid palette URL");
            let palette_window =
                WebviewWindowBuilder::new(app, "palette", WebviewUrl::External(palette_url))
                    .title("")
                    .inner_size(680.0, 480.0)
                    .decorations(false)
                    .always_on_top(true)
                    .resizable(false)
                    .center()
                    .visible(false)
                    .build()?;
            let pw = palette_window.clone();
            palette_window.on_window_event(move |event| {
                if let tauri::WindowEvent::Focused(false) = event {
                    pw.hide().ok();
                }
            });

            // Register Cmd+Shift+Space to toggle the palette.
            use tauri_plugin_global_shortcut::GlobalShortcutExt;
            app.global_shortcut().register(hotkey.as_str())?;

            build_tray(app, sp_tray, port, log.clone())?;

            // Background sync loop — interval is re-read from config each cycle.
            tauri::async_runtime::spawn(async move {
                let _ = app_handle; // keep handle alive
                loop {
                    let config = memoir::Settings::load();
                    let secs = config.sync.interval_mins.max(1) * 60;
                    tokio::time::sleep(std::time::Duration::from_secs(secs)).await;

                    if sp_loop.load(Ordering::Relaxed) {
                        continue;
                    }

                    let embedder = if config.embed.enabled {
                        let cache = memoir::Settings::config_dir().join("models");
                        let model = config.embed.model;
                        match tokio::task::spawn_blocking(move || {
                            memoir::Embedder::try_new(cache, model)
                        })
                        .await
                        {
                            Ok(Ok(e)) => Some(Arc::new(e) as Arc<dyn memoir::EmbedText>),
                            Ok(Err(err)) => {
                                tracing::warn!(error = %err, "embedding model unavailable");
                                None
                            }
                            Err(_) => None,
                        }
                    } else {
                        None
                    };

                    if let Err(e) = memoir::sync::run(&config, embedder, Some(log.clone())).await {
                        tracing::warn!(error = %e, "scheduled sync failed");
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running memoir");
}

/// Show (or recreate) the main window.
/// If `focus_search` is true, eval JS to focus the search input; if the user
/// is on a page without a search box, navigate home first.
fn show_main_window(app: &tauri::AppHandle, port: u16, focus_search: bool) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
        if focus_search {
            let _ = w.eval(
                "var q=document.getElementById('q');\
                 if(q){q.focus();}else{window.location.href='/';}",
            );
        }
    } else {
        let config_exists = memoir::Settings::config_dir().join("config.toml").exists();
        let path = if config_exists { "/" } else { "/setup" };
        let url: url::Url = format!("http://127.0.0.1:{port}{path}")
            .parse()
            .expect("valid URL");
        let _ = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
            .title("Memoir")
            .inner_size(1280.0, 820.0)
            .min_inner_size(800.0, 600.0)
            .initialization_script(INIT_SCRIPT)
            .build();
    }
}

fn build_tray(
    app: &tauri::App,
    sync_paused: Arc<AtomicBool>,
    port: u16,
    log: Arc<memoir::SessionLog>,
) -> tauri::Result<()> {
    use tauri_plugin_autostart::ManagerExt;

    let is_autolaunching = app.autolaunch().is_enabled().unwrap_or(false);
    let autolaunch_label = if is_autolaunching {
        "✓ Launch at Login"
    } else {
        "Launch at Login"
    };

    let open = MenuItem::with_id(app, "open", "Open Memoir", true, None::<&str>)?;
    let search = MenuItem::with_id(app, "search", "Search…", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let index_now = MenuItem::with_id(app, "index_now", "Index Now", true, None::<&str>)?;
    let pause_idx = MenuItem::with_id(app, "pause_index", "Pause Indexing", true, None::<&str>)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let autolaunch = MenuItem::with_id(app, "autolaunch", autolaunch_label, true, None::<&str>)?;
    let check_updates = MenuItem::with_id(
        app,
        "check_updates",
        "Check for Updates…",
        true,
        None::<&str>,
    )?;
    let sep3 = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &open,
            &search,
            &sep1,
            &index_now,
            &pause_idx,
            &sep2,
            &autolaunch,
            &check_updates,
            &sep3,
            &quit,
        ],
    )?;

    let pause_item = pause_idx.clone();
    let autolaunch_item = autolaunch.clone();
    let check_updates_item = check_updates.clone();

    TrayIconBuilder::new()
        .tooltip("Memoir")
        .icon(tauri::include_image!("icons/tray-icon.png"))
        .menu(&menu)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "open" => show_main_window(app, port, false),

            "search" => show_main_window(app, port, true),

            "index_now" => {
                let log_tray = log.clone();
                tauri::async_runtime::spawn(async move {
                    let config = memoir::Settings::load();
                    let embedder = if config.embed.enabled {
                        let cache = memoir::Settings::config_dir().join("models");
                        let model = config.embed.model;
                        match tokio::task::spawn_blocking(move || memoir::Embedder::try_new(cache, model)).await {
                            Ok(Ok(e)) => Some(Arc::new(e) as Arc<dyn memoir::EmbedText>),
                            Ok(Err(err)) => {
                                tracing::warn!(error = %err, "embedding model unavailable");
                                None
                            }
                            Err(_) => None,
                        }
                    } else {
                        None
                    };
                    if let Err(e) = memoir::sync::run(&config, embedder, Some(log_tray)).await {
                        tracing::warn!(error = %e, "manual index failed");
                    }
                });
            }

            "pause_index" => {
                let was_paused = sync_paused.fetch_xor(true, Ordering::SeqCst);
                let new_label = if was_paused {
                    "Pause Indexing"
                } else {
                    "Resume Indexing"
                };
                let _ = pause_item.set_text(new_label);
            }

            "autolaunch" => {
                use tauri_plugin_autostart::ManagerExt;
                let al = app.autolaunch();
                let enabled = al.is_enabled().unwrap_or(false);
                if enabled {
                    let _ = al.disable();
                    let _ = autolaunch_item.set_text("Launch at Login");
                } else {
                    let _ = al.enable();
                    let _ = autolaunch_item.set_text("✓ Launch at Login");
                }
            }

            "check_updates" => {
                use tauri_plugin_updater::UpdaterExt;
                let app = app.clone();
                let item = check_updates_item.clone();
                let log_upd = log.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = item.set_text("Checking…");
                    log_upd.push(memoir::LogKind::Sync, "Update check started", None);
                    match app.updater() {
                        Ok(updater) => match updater.check().await {
                            Ok(Some(update)) => {
                                tracing::info!(version = %update.version, "update available, installing");
                                let _ = item.set_text("Updating…");
                                log_upd.push(
                                    memoir::LogKind::Sync,
                                    format!("Update available: v{}", update.version),
                                    None,
                                );
                                match update.download_and_install(|_, _| {}, || {}).await {
                                    Ok(()) => {
                                        tracing::info!("update installed, restarting");
                                        log_upd.push(memoir::LogKind::Sync, "Update installed, restarting", None);
                                        app.restart();
                                    }
                                    Err(e) => {
                                        tracing::warn!(error = %e, "update install failed");
                                        log_upd.push(memoir::LogKind::Error, "Update install failed", e.to_string());
                                        let _ = item.set_text("Update failed");
                                        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                                        let _ = item.set_text("Check for Updates…");
                                    }
                                }
                            }
                            Ok(None) => {
                                tracing::info!("memoir is up to date");
                                log_upd.push(memoir::LogKind::Sync, "Already up to date", None);
                                let _ = item.set_text("✓ Up to date");
                                tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                                let _ = item.set_text("Check for Updates…");
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "update check failed");
                                log_upd.push(memoir::LogKind::Error, "Update check failed", e.to_string());
                                let _ = item.set_text("Check failed");
                                tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                                let _ = item.set_text("Check for Updates…");
                            }
                        },
                        Err(e) => {
                            tracing::warn!(error = %e, "updater unavailable");
                            log_upd.push(memoir::LogKind::Error, "Updater unavailable", e.to_string());
                            let _ = item.set_text("Check for Updates…");
                        }
                    }
                });
            }

            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}
