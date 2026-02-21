use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, RwLock};

use crate::translator::Translator;

#[derive(Clone, Copy)]
pub struct SelectionPos {
    pub down_x: i32,
    pub down_y: i32,
    pub up_x: i32,
    pub up_y: i32,
}

pub struct TranslationRequest {
    pub text: String,
    pub pos: SelectionPos,
}

pub struct TranslationResult {
    pub original: String,
    pub translated: String,
    pub pos: SelectionPos,
}

pub fn spawn_translation_thread(
    text_rx: Receiver<TranslationRequest>,
    result_tx: Sender<TranslationResult>,
    api_url: String,
    api_key: Option<String>,
    source_lang: String,
    target_lang: Arc<RwLock<String>>,
    server_status: Arc<AtomicU8>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build tokio runtime");

        rt.block_on(async move {
            let is_local = api_url.contains("localhost") || api_url.contains("127.0.0.1");
            let translator = Translator::new(api_url, api_key, source_lang, target_lang);

            while let Ok(req) = text_rx.recv() {
                match translator.translate(&req.text).await {
                    Ok(translated) => {
                        tracing::info!("Translation complete");
                        let _ = result_tx.send(TranslationResult {
                            original: req.text,
                            translated,
                            pos: req.pos,
                        });
                    }
                    Err(e) => {
                        tracing::error!("Translation failed: {}", e);

                        let status = server_status.load(Ordering::Relaxed);
                        let error_str = e.to_string();

                        let error_msg = if status == crate::server::SERVER_FAILED {
                            "⚠️ LibreTranslate failed to start\n\
                             Check libretranslate.log in app data folder"
                                .to_string()
                        } else if is_local {
                            let is_conn_error = error_str.contains("Connection refused")
                                || error_str.contains("connect")
                                || error_str.contains("timed out")
                                || error_str.contains("timeout")
                                || error_str.contains("500")
                                || error_str.contains("503")
                                || error_str.contains("model");

                            if is_conn_error
                                && status == crate::server::SERVER_STARTING
                            {
                                "⏳ LibreTranslate is loading...\n\
                                 First launch may take a few minutes\n\
                                 to download language models"
                                    .to_string()
                            } else if is_conn_error {
                                "⚠️ Cannot connect to LibreTranslate\n\
                                 Server may have crashed.\n\
                                 Check libretranslate.log for details"
                                    .to_string()
                            } else {
                                "⚠️ Translation Unavailable\n\
                                 Check if app installed correctly"
                                    .to_string()
                            }
                        } else {
                            format!("⚠️ API Error:\n{}", e)
                        };

                        let _ = result_tx.send(TranslationResult {
                            original: req.text,
                            translated: error_msg,
                            pos: req.pos,
                        });
                    }
                }
            }
        });
    })
}
