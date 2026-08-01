//! Anthropic プロバイダ（仕様書 7.2）。
//!
//! `POST https://api.anthropic.com/v1/messages`
//! ヘッダは `x-api-key` と `anthropic-version: 2023-06-01`。
//! system は messages ではなく独立フィールド。

use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::json;

use super::{
    classify_status, credentials, http_client, ChatMessage, CompletionRequest, CompletionResponse,
    LlmError, LlmProvider, Provider, VERIFY_TIMEOUT,
};

const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// 既定モデル。設定で上書きできる（仕様書 7.2「ハードコードしない」）。
pub const DEFAULT_MODEL: &str = "claude-sonnet-5";

pub struct Anthropic {
    model: String,
}

impl Anthropic {
    pub fn new(model: impl Into<String>) -> Self {
        Anthropic {
            model: model.into(),
        }
    }

    pub fn with_default_model() -> Self {
        Self::new(DEFAULT_MODEL)
    }

    async fn post(
        &self,
        body: serde_json::Value,
        timeout: Duration,
    ) -> Result<(u16, String, u64), LlmError> {
        let client = http_client(timeout)?;

        // キーはこのクロージャの外に出さない（仕様書 7.5.2）。
        let request = credentials::with_key(Provider::Anthropic, |key| {
            client
                .post(ENDPOINT)
                .header("x-api-key", key)
                .header("anthropic-version", API_VERSION)
                .header("content-type", "application/json")
                .json(&body)
        })
        .map_err(|e| LlmError::Auth(e.to_string()))?;

        // 計測はここから。鍵の取り出しは Keychain の許可待ちで
        // 何分も止まりうるので、含めるとレイテンシが意味を失う。
        let started = Instant::now();
        let response = request.send().await.map_err(|e| {
            // エラー表示にヘッダを含めない（x-api-key が乗るため）。
            LlmError::Network(e.without_url().to_string())
        })?;

        let status = response.status().as_u16();
        let text = response
            .text()
            .await
            .map_err(|e| LlmError::Network(e.without_url().to_string()))?;
        Ok((status, text, started.elapsed().as_millis() as u64))
    }
}

#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct Usage {
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
}

fn to_json_messages(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|m| json!({ "role": m.role, "content": m.content }))
        .collect()
}

#[async_trait::async_trait]
impl LlmProvider for Anthropic {
    fn id(&self) -> &'static str {
        "anthropic"
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let body = json!({
            "model": req.model,
            "system": req.system,
            "messages": to_json_messages(&req.messages),
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
        });

        let (status, text, latency_ms) = self.post(body, REQUEST_TIMEOUT).await?;
        if status != 200 {
            return Err(classify_status(status, &text));
        }

        let parsed: MessagesResponse = serde_json::from_str(&text)
            .map_err(|e| LlmError::InvalidOutput(format!("JSON を解釈できない: {e}")))?;

        let output = parsed
            .content
            .iter()
            .filter(|b| b.kind == "text")
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("");

        if output.trim().is_empty() {
            return Err(LlmError::InvalidOutput("応答が空".into()));
        }

        Ok(CompletionResponse {
            text: output,
            input_tokens: parsed.usage.as_ref().and_then(|u| u.input_tokens),
            output_tokens: parsed.usage.as_ref().and_then(|u| u.output_tokens),
            latency_ms,
        })
    }

    /// 仕様書 7.5.5: `max_tokens: 1` で `"hi"` を 1 回送る。
    async fn verify(&self) -> Result<(), LlmError> {
        let body = json!({
            "model": self.model,
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 1,
        });

        let (status, text, _) = self.post(body, VERIFY_TIMEOUT).await?;
        if status == 200 {
            return Ok(());
        }
        Err(classify_status(status, &text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_keep_their_roles() {
        let msgs = vec![
            ChatMessage::user("こんにちは"),
            ChatMessage::assistant("やあ"),
        ];
        let json = to_json_messages(&msgs);
        assert_eq!(json[0]["role"], "user");
        assert_eq!(json[1]["role"], "assistant");
        assert_eq!(json[1]["content"], "やあ");
    }

    /// content は複数ブロックに分かれて返ることがある。
    #[test]
    fn text_blocks_are_concatenated() {
        let raw = r#"{
            "content": [
                {"type": "text", "text": "前半"},
                {"type": "thinking", "text": "無視される"},
                {"type": "text", "text": "後半"}
            ],
            "usage": {"input_tokens": 12, "output_tokens": 3}
        }"#;
        let parsed: MessagesResponse = serde_json::from_str(raw).unwrap();
        let joined = parsed
            .content
            .iter()
            .filter(|b| b.kind == "text")
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(joined, "前半後半");
        assert_eq!(parsed.usage.unwrap().input_tokens, Some(12));
    }

    #[test]
    fn usage_may_be_absent() {
        let raw = r#"{"content": [{"type":"text","text":"ok"}]}"#;
        let parsed: MessagesResponse = serde_json::from_str(raw).unwrap();
        assert!(parsed.usage.is_none());
    }
}
