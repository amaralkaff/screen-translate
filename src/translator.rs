use std::sync::{Arc, RwLock};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct TranslateRequest<'a> {
    q: &'a str,
    source: &'a str,
    target: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_key: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TranslateResponse {
    translated_text: String,
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: String,
}

pub struct Translator {
    client: reqwest::Client,
    api_url: String,
    api_key: Option<String>,
    source_lang: String,
    target_lang: Arc<RwLock<String>>,
}

impl Translator {
    pub fn new(
        api_url: String,
        api_key: Option<String>,
        source_lang: String,
        target_lang: Arc<RwLock<String>>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            api_url,
            api_key,
            source_lang,
            target_lang,
        }
    }

    pub async fn translate(&self, text: &str) -> Result<String> {
        let target = self.target_lang.read().unwrap().clone();
        let body = TranslateRequest {
            q: text,
            source: &self.source_lang,
            target: &target,
            api_key: self.api_key.as_deref(),
        };

        let resp = self.client.post(&self.api_url).json(&body).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if let Ok(err) = serde_json::from_str::<ErrorResponse>(&text) {
                anyhow::bail!("LibreTranslate error ({}): {}", status, err.error);
            }
            anyhow::bail!("LibreTranslate HTTP {}: {}", status, text);
        }

        let result: TranslateResponse = resp.json().await?;
        Ok(result.translated_text)
    }
}
