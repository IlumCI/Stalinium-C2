use reqwest::Client;
use std::error::Error;

use super::codec::*;

pub struct AgenticClient {
    client: Client,
    endpoint: String,
    api_key: String,
}

impl AgenticClient {
    pub fn new(endpoint: &str, api_key: &str) -> Self {
        let client = Client::builder()
            .user_agent("OpenAI-Python/1.30.0")
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            endpoint: endpoint.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
        }
    }

    pub async fn send_as_chat_completion(
        &self,
        id: usize,
        command: &str,
        args: &[String],
    ) -> Result<(usize, String), Box<dyn Error + Send + Sync>> {
        let req_body = encode_command(id, command, args);

        let resp = self
            .client
            .post(format!("{}/v1/chat/completions", self.endpoint))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&req_body)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(format!("server returned {}", resp.status()).into());
        }

        let chat_resp: ChatCompletionResponse = resp.json().await?;
        decode_result(&chat_resp)
    }

    pub async fn poll_via_models(
        &self,
    ) -> Result<Option<serde_json::Value>, Box<dyn Error + Send + Sync>> {
        let resp = self
            .client
            .get(format!("{}/v1/models", self.endpoint))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(format!("server returned {}", resp.status()).into());
        }

        let body: serde_json::Value = resp.json().await?;
        Ok(body.get("x_metadata").cloned())
    }
}
