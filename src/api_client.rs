use std::path::Path;
use std::time::Duration;

use md5::{Digest, Md5};
use reqwest::{multipart, Client};
use tracing::{debug, warn};

use crate::error::ApiError;
use crate::types::ApiResponse;

const MAX_RETRIES: u32 = 3;

pub struct ApiClient {
    client: Client,
    base_url: String,
}

impl ApiClient {
    pub fn new(base_url: &str) -> Result<Self, ApiError> {
        let client = Client::builder()
            .build()
            .map_err(ApiError::Request)?;

        let base_url = base_url.trim_end_matches('/').to_string();
        Ok(Self { client, base_url })
    }

    pub async fn convert(&self, pdf_path: &Path, timeout: Duration) -> Result<ApiResponse, ApiError> {
        let pdf_bytes = tokio::fs::read(pdf_path)
            .await
            .map_err(|e| ApiError::Server(format!("Failed to read PDF: {e}")))?;

        let filename = pdf_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let mut last_error = None;
        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                let backoff = Duration::from_secs(1 << attempt);
                warn!(attempt, backoff_secs = backoff.as_secs(), "Retrying...");
                tokio::time::sleep(backoff).await;
            }

            match self.try_convert(&pdf_bytes, &filename, timeout).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    if !is_retryable(&e) {
                        return Err(e);
                    }
                    debug!(attempt, error = %e, "Retryable error");
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap())
    }

    async fn try_convert(
        &self,
        pdf_bytes: &[u8],
        filename: &str,
        timeout: Duration,
    ) -> Result<ApiResponse, ApiError> {
        let md5_hash = format!("{:x}", Md5::digest(pdf_bytes));

        let part = multipart::Part::bytes(pdf_bytes.to_vec())
            .file_name(filename.to_string())
            .mime_str("application/pdf")
            .map_err(ApiError::Request)?;

        let form = multipart::Form::new().part("file", part);

        let url = format!("{}/api/parse", self.base_url);
        let response = self
            .client
            .post(&url)
            .header("X-File-MD5", &md5_hash)
            .multipart(form)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ApiError::Timeout(timeout.as_secs())
                } else {
                    ApiError::Request(e)
                }
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ApiError::Server(format!("HTTP {status}: {body}")));
        }

        let body = response.text().await.map_err(ApiError::Request)?;
        let api_resp: ApiResponse = serde_json::from_str(&body)
            .map_err(|e| ApiError::InvalidResponse(format!("{e}: {body}")))?;

        Ok(api_resp)
    }

    pub async fn health_check(&self) -> Result<bool, ApiError> {
        let url = format!("{}/health", self.base_url);
        let response = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(ApiError::Request)?;

        Ok(response.status().is_success())
    }
}

fn is_retryable(error: &ApiError) -> bool {
    matches!(error, ApiError::Request(_))
}
