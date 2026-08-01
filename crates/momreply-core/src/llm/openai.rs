//! OpenAI プロバイダ（仕様書 7.2）。
//!
//! `POST https://api.openai.com/v1/chat/completions`
//! 認証は `Authorization: Bearer`。system は独立フィールドではなく
//! `messages` の先頭に置く。

use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::json;

use super::{
    classify_status, credentials, http_client, ChatMessage, CompletionRequest, CompletionResponse,
    LlmError, LlmProvider, Provider, VERIFY_TIMEOUT,
};

const ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// 既定モデル。設定で上書きできる（仕様書 7.2「ハードコードしない」）。
///
/// **この名前が現在も有効かは疎通テストで確かめること。**
pub const DEFAULT_MODEL: &str = "gpt-5";

pub struct Openai {
    model: String,
}

impl Openai {
    pub fn new(model: impl Into<String>) -> Self {
        Openai {
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
    ) -> Result<(u16, String), LlmError> {
        let client = http_client(timeout)?;

        let request = credentials::with_key(Provider::Openai, |key| {
            client
                .post(ENDPOINT)
                .header("authorization", format!("Bearer {key}"))
                .header("content-type", "application/json")
                .json(&body)
        })
        .map_err(|e| LlmError::Auth(e.to_string()))?;

        let response = request
            .send()
            .await
            .map_err(|e| LlmError::Network(e.without_url().to_string()))?;

        let status = response.status().as_u16();
        let text = response
            .text()
            .await
            .map_err(|e| LlmError::Network(e.without_url().to_string()))?;
        Ok((status, text))
    }
}

#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Choice {
    message: Option<Message>,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct Message {
    content: Option<String>,
}

#[derive(Deserialize)]
struct Usage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
}

/// system を先頭に差し込んだ messages を作る。
fn to_messages(system: &str, messages: &[ChatMessage]) -> Vec<serde_json::Value> {
    let mut out = Vec::with_capacity(messages.len() + 1);
    if !system.trim().is_empty() {
        out.push(json!({ "role": "system", "content": system }));
    }
    out.extend(
        messages
            .iter()
            .map(|m| json!({ "role": m.role, "content": m.content })),
    );
    out
}

fn extract_text(parsed: &ChatResponse) -> String {
    parsed
        .choices
        .first()
        .and_then(|c| c.message.as_ref())
        .and_then(|m| m.content.clone())
        .unwrap_or_default()
}

#[async_trait::async_trait]
impl LlmProvider for Openai {
    fn id(&self) -> &'static str {
        "openai"
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let started = Instant::now();
        // 新しめのモデルは max_tokens を受け付けず max_completion_tokens を要求する。
        let body = json!({
            "model": req.model,
            "messages": to_messages(&req.system, &req.messages),
            "max_completion_tokens": req.max_tokens,
            "temperature": req.temperature,
        });

        let (status, text) = self.post(body, REQUEST_TIMEOUT).await?;
        if status != 200 {
            return Err(classify_status(status, &text));
        }

        let parsed: ChatResponse = serde_json::from_str(&text)
            .map_err(|e| LlmError::InvalidOutput(format!("JSON を解釈できない: {e}")))?;

        let output = extract_text(&parsed);
        if output.trim().is_empty() {
            let reason = parsed
                .choices
                .first()
                .and_then(|c| c.finish_reason.clone())
                .unwrap_or_else(|| "不明".into());
            return Err(LlmError::InvalidOutput(format!(
                "応答が空（finish_reason: {reason}）"
            )));
        }

        Ok(CompletionResponse {
            text: output,
            input_tokens: parsed.usage.as_ref().and_then(|u| u.prompt_tokens),
            output_tokens: parsed.usage.as_ref().and_then(|u| u.completion_tokens),
            latency_ms: started.elapsed().as_millis() as u64,
        })
    }

    /// 仕様書 7.5.5: 最小トークンで 1 回送る。
    ///
    /// temperature は載せない。モデルによっては既定値以外を拒否するため、
    /// キーの検証が温度設定のせいで落ちるのを避ける。
    async fn verify(&self) -> Result<(), LlmError> {
        let body = json!({
            "model": self.model,
            "messages": [{ "role": "user", "content": "hi" }],
            "max_completion_tokens": 1,
        });

        let (status, text) = self.post(body, VERIFY_TIMEOUT).await?;
        if status == 200 {
            return Ok(());
        }
        Err(classify_status(status, &text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// system は独立フィールドではなく messages の先頭。
    #[test]
    fn system_goes_first_in_messages() {
        let msgs = vec![ChatMessage::user("やあ")];
        let out = to_messages("あなたは本人です", &msgs);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["role"], "system");
        assert_eq!(out[0]["content"], "あなたは本人です");
        assert_eq!(out[1]["role"], "user");
    }

    #[test]
    fn an_empty_system_adds_no_message() {
        let out = to_messages("   ", &[ChatMessage::user("やあ")]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["role"], "user");
    }

    #[test]
    fn assistant_role_is_passed_through() {
        // Gemini と違い OpenAI は assistant をそのまま使う。
        let out = to_messages("", &[ChatMessage::assistant("うん")]);
        assert_eq!(out[0]["role"], "assistant");
    }

    #[test]
    fn content_is_read_from_the_first_choice() {
        let raw = r#"{
            "choices": [{"message": {"role": "assistant", "content": "やあ"},
                         "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 2}
        }"#;
        let parsed: ChatResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(extract_text(&parsed), "やあ");
        assert_eq!(parsed.usage.unwrap().prompt_tokens, Some(10));
    }

    /// 推論に枠を使い切ると content が null で返る。panic させない。
    #[test]
    fn a_null_content_is_not_a_panic() {
        let raw = r#"{"choices": [{"message": {"content": null},
                                   "finish_reason": "length"}]}"#;
        let parsed: ChatResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(extract_text(&parsed), "");
        assert_eq!(parsed.choices[0].finish_reason.as_deref(), Some("length"));
    }

    #[test]
    fn an_empty_choice_list_is_not_a_panic() {
        let parsed: ChatResponse = serde_json::from_str(r#"{"choices": []}"#).unwrap();
        assert_eq!(extract_text(&parsed), "");
    }
}
