#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod clipboard;
mod config;
mod platform;
mod server;
mod translator;
mod tray;
mod updater;

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use clipboard::{SelectionPos, TranslationRequest, TranslationResult};
use platform::MouseEvent;
use tray::TrayAction;

fn setup_logging() {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::fmt;

    let log_dir = config::Config::app_dir();
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("debug.log");

    // Truncate to keep log file manageable (overwrite each launch)
    let file = std::fs::File::create(&log_path).ok();

    let stdout_layer = fmt::layer()
        .with_target(false)
        .with_writer(std::io::stdout);

    if let Some(file) = file {
        let file_layer = fmt::layer()
            .with_target(false)
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(file));

        tracing_subscriber::registry()
            .with(tracing_subscriber::filter::LevelFilter::INFO)
            .with(stdout_layer)
            .with(file_layer)
            .init();
    } else {
        tracing_subscriber::registry()
            .with(tracing_subscriber::filter::LevelFilter::INFO)
            .with(stdout_layer)
            .init();
    }
}

fn main() {
    setup_logging();

    std::panic::set_hook(Box::new(|info| {
        let msg = format!("{}", info);
        tracing::error!("PANIC: {}", msg);
        platform::show_error("Screen Translate crashed", &msg);
    }));

    platform::init_platform();

    tracing::info!("Screen Translate starting");

    let config = config::Config::load().unwrap_or_else(|e| {
        tracing::warn!("Failed to load config: {}, using defaults", e);
        config::Config::default()
    });

    updater::cleanup_old_binary();

    enum UpdateNotification {
        UpToDate,
        Available(updater::UpdateInfo),
    }

    let update_notify: Arc<Mutex<Option<UpdateNotification>>> = Arc::new(Mutex::new(None));

    if config.auto_update {
        let notify = update_notify.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(5));
            match updater::check_for_update() {
                Ok(Some(info)) => {
                    tracing::info!("Update available: v{}", info.version);
                    *notify.lock().unwrap() = Some(UpdateNotification::Available(info));
                }
                Ok(None) => {}
                Err(e) => tracing::debug!("Update check: {}", e),
            }
        });
    }

    let target_lang = Arc::new(RwLock::new(config.target_lang.clone()));
    let languages: Vec<String> = config
        .load_languages
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|c| c != "en")
        .collect();

    // Create tray icon FIRST so user sees the app is running
    let tray = match tray::Tray::new(&languages, &config.target_lang) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Failed to create tray icon: {}", e);
            platform::show_error(
                "Screen Translate",
                &format!("Failed to create system tray icon: {}\n\nThe application cannot continue.", e),
            );
            return;
        }
    };

    let mut actual_api_url = config.api_url.clone();

    // Track server readiness so we can show accurate error messages
    let server_status = Arc::new(AtomicU8::new(server::SERVER_READY));

    let _server = if !config.start_local_server {
        tracing::info!("Local server disabled, using external API: {}", config.api_url);
        None
    } else if server::is_libretranslate_running(config.api_port) {
        tracing::info!("LibreTranslate already running on port {}", config.api_port);
        None
    } else {
        // Start LibreTranslate in background - don't block UI!
        server_status.store(server::SERVER_STARTING, Ordering::Relaxed);
        tracing::info!("Starting LibreTranslate in background...");
        match server::LibreTranslateServer::start_background(
            config.python_path.as_deref(),
            config.api_port,
            &config.load_languages,
        ) {
            Ok(s) => {
                let actual_port = s.port();
                if actual_port != config.api_port {
                    actual_api_url = format!("http://127.0.0.1:{}/translate", actual_port);
                    tracing::info!("Updated API URL to: {}", actual_api_url);
                }
                // Monitor process health and readiness in background
                server::spawn_readiness_monitor(actual_port, s.child_handle(), server_status.clone());
                tracing::info!("App ready - LibreTranslate starting on port {}", actual_port);
                Some(s)
            }
            Err(e) => {
                tracing::error!("Failed to start LibreTranslate: {}", e);
                server_status.store(server::SERVER_FAILED, Ordering::Relaxed);
                tracing::info!("Translations will fail. To fix: install LibreTranslate or set start_local_server=false");
                None
            }
        }
    };

    let monitoring = std::sync::Arc::new(AtomicBool::new(true));

    let (text_tx, text_rx) = mpsc::channel::<TranslationRequest>();
    let (result_tx, result_rx) = mpsc::channel::<TranslationResult>();

    let _translation_handle = clipboard::spawn_translation_thread(
        text_rx,
        result_tx,
        actual_api_url,
        config.api_key,
        config.source_lang,
        target_lang.clone(),
        server_status,
    );

    // grab thread — reads clipboard off the main thread
    let (grab_tx, grab_rx) = mpsc::channel::<SelectionPos>();
    let text_tx_clone = text_tx.clone();
    let max_text_length = config.max_text_length;
    std::thread::spawn(move || {
        let mut last_text = String::new();
        while let Ok(pos) = grab_rx.recv() {
            let mut pos = pos;
            while let Ok(newer) = grab_rx.try_recv() {
                pos = newer;
            }

            if let Some(text) = grab_selection() {
                let trimmed = text.trim().to_string();
                if trimmed.len() >= 2 && trimmed.len() <= max_text_length && trimmed != last_text {
                    let preview: String = trimmed.chars().take(50).collect();
                    tracing::info!("Selection: \"{}\"", preview);
                    last_text = trimmed.clone();
                    let _ = text_tx_clone.send(TranslationRequest { text: trimmed, pos });
                }
            }
        }
    });

    let _hook = match platform::install_mouse_hook() {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("{}", e);
            platform::show_error(
                "Screen Translate — Permission Required",
                &format!("{}\n\nThe app will now exit.", e),
            );
            return;
        }
    };

    let mut debounce_start: Option<Instant> = None;
    let mut pending_pos = SelectionPos { down_x: 0, down_y: 0, up_x: 0, up_y: 0 };
    let debounce_ms = Duration::from_millis(config.poll_interval_ms.max(50));
    let mut last_click_time: Option<Instant> = None;
    let mut last_click_x: i32 = 0;
    let mut last_click_y: i32 = 0;
    let dblclick_ms = platform::get_double_click_time_ms();

    let monitoring_ref = monitoring.clone();
    loop {
        while let Some(event) = platform::poll_mouse_event() {
            match event {
                MouseEvent::Quit => {
                    tracing::info!("Screen Translate exiting");
                    return;
                }
                MouseEvent::SelectionDone { down_x, down_y, up_x, up_y } => {
                    if monitoring_ref.load(Ordering::Relaxed) {
                        pending_pos = SelectionPos { down_x, down_y, up_x, up_y };
                        let dx = (up_x - down_x).abs();
                        let dy = (up_y - down_y).abs();

                        if dx > 5 || dy > 5 {
                            debounce_start = Some(Instant::now());
                        } else {
                            let now = Instant::now();
                            let is_dblclick = if let Some(prev) = last_click_time {
                                let dt = now.duration_since(prev).as_millis() as u64;
                                let cx = (up_x - last_click_x).abs();
                                let cy = (up_y - last_click_y).abs();
                                dt <= dblclick_ms && cx < 10 && cy < 10
                            } else {
                                false
                            };

                            last_click_time = Some(now);
                            last_click_x = up_x;
                            last_click_y = up_y;

                            if is_dblclick {
                                debounce_start = Some(Instant::now());
                            }
                        }
                    }
                }
                MouseEvent::Click => {
                    platform::on_click_away();
                }
            }
        }

        if let Some(start) = debounce_start {
            if start.elapsed() >= debounce_ms {
                debounce_start = None;
                let _ = grab_tx.send(pending_pos);
            }
        }

        while let Ok(result) = result_rx.try_recv() {
            let orig_preview: String = result.original.chars().take(40).collect();
            let trans_preview: String = result.translated.chars().take(40).collect();
            tracing::info!("\"{}\" -> \"{}\"", orig_preview, trans_preview);
            platform::show_popup(
                &result.original,
                &result.translated,
                config.popup_duration_secs,
                result.pos,
            );
        }

        if let Some(notification) = update_notify.lock().unwrap().take() {
            match notification {
                UpdateNotification::UpToDate => {
                    platform::show_info(
                        "Screen Translate",
                        &format!(
                            "You're running the latest version (v{}).",
                            env!("CARGO_PKG_VERSION")
                        ),
                    );
                }
                UpdateNotification::Available(info) => {
                    tray.set_update_in_progress();
                    tracing::info!("Auto-installing update v{}...", info.version);
                    std::thread::spawn(move || {
                        if let Err(e) = updater::perform_update(&info) {
                            tracing::error!("Update failed: {}", e);
                        }
                    });
                }
            }
        }

        match tray.handle_menu_event() {
            TrayAction::Quit => {
                tracing::info!("Quit requested");
                break;
            }
            TrayAction::ToggleMonitoring(active) => {
                monitoring.store(active, Ordering::Relaxed);
                tracing::info!("Monitoring: {}", active);
            }
            TrayAction::ChangeLanguage(code) => {
                *target_lang.write().unwrap() = code.clone();
                tracing::info!("Target language changed to: {}", code);
            }
            TrayAction::CheckForUpdates => {
                let notify = update_notify.clone();
                std::thread::spawn(move || {
                    match updater::check_for_update() {
                        Ok(Some(info)) => {
                            tracing::info!("Update available: v{}", info.version);
                            *notify.lock().unwrap() = Some(UpdateNotification::Available(info));
                        }
                        Ok(None) => {
                            *notify.lock().unwrap() = Some(UpdateNotification::UpToDate);
                        }
                        Err(e) => tracing::error!("Update check failed: {}", e),
                    }
                });
            }
            TrayAction::None => {}
        }

        std::thread::sleep(Duration::from_millis(10));
    }

    tracing::info!("Screen Translate exiting");
}

fn grab_selection() -> Option<String> {
    let mut clip = arboard::Clipboard::new().ok()?;
    let _ = clip.set_text(String::new());

    platform::send_copy_command();
    std::thread::sleep(Duration::from_millis(80));

    let new_text = clip.get_text().ok();

    match new_text {
        Some(t) if !t.is_empty() => Some(t),
        _ => None,
    }
}
