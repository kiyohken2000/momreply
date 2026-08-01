//! Gemini プロバイダ（仕様書 7.2）。
//!
//! `POST https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent`
//! 認証は `x-goog-api-key`。system は `systemInstruction` という独立フィールドで、
//! role は `user` / `model`（`assistant` ではない）。

use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::json;

use super::{
    classify_status, credentials, http_client, ChatMessage, CompletionRequest, CompletionResponse,
    LlmError, LlmProvider, Provider, VERIFY_TIMEOUT,
};

const BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// 推論・思考のための余白。
///
/// 推論モデルは本文を書く前に内部でトークンを使う。要求した文字数ぶん
/// しか枠を与えないと、本文に到達する前に上限へ当たり、finish_reason が
/// length で中身が空のまま返る。実機で踏んだ。
///
/// **上限は費用ではなく天井**なので、広く取っても実際に使った分しか
/// 課金されない。長さの暴走は hard_max_length 側で止める。
const REASONING_HEADROOM: u32 = 6000;

/// 既定モデル。設定で上書きできる（仕様書 7.2「ハードコードしない」）。
///
/// **この名前が現在も有効かは疎通テストで確かめること。** 存在しない
/// モデル名は 404 で返るので、UI にそのまま理由が出る。
pub const DEFAULT_MODEL: &str = "gemini-2.5-flash";

pub struct Gemini {
    model: String,
}

impl Gemini {
    pub fn new(model: impl Into<String>) -> Self {
        Gemini {
            model: model.into(),
        }
    }

    pub fn with_default_model() -> Self {
        Self::new(DEFAULT_MODEL)
    }

    async fn post(
        &self,
        model: &str,
        body: serde_json::Value,
        timeout: Duration,
    ) -> Result<(u16, String, u64), LlmError> {
        let client = http_client(timeout)?;
        let url = format!("{BASE}/{model}:generateContent");

        // キーはクエリ文字列ではなくヘッダで渡す。
        // URL に載せるとログやエラーメッセージに混入しやすい。
        let request = credentials::with_key(Provider::Gemini, |key| {
            client
                .post(&url)
                .header("x-goog-api-key", key)
                .header("content-type", "application/json")
                .json(&body)
        })
        .map_err(|e| LlmError::Auth(e.to_string()))?;

        // 計測はここから。鍵の取り出しは Keychain の許可待ちで
        // 何分も止まりうるので、含めるとレイテンシが意味を失う。
        let started = Instant::now();
        let response = request
            .send()
            .await
            .map_err(|e| LlmError::Network(e.without_url().to_string()))?;

        let status = response.status().as_u16();
        let text = response
            .text()
            .await
            .map_err(|e| LlmError::Network(e.without_url().to_string()))?;
        Ok((status, text, started.elapsed().as_millis() as u64))
    }
}

#[derive(Deserialize)]
struct GenerateResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<UsageMetadata>,
}

#[derive(Deserialize)]
struct Candidate {
    content: Option<Content>,
    #[serde(rename = "finishReason")]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct Content {
    #[serde(default)]
    parts: Vec<Part>,
}

#[derive(Deserialize)]
struct Part {
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct UsageMetadata {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: Option<u32>,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: Option<u32>,
}

/// `assistant` を `model` に読み替える。Gemini はこの名前しか受け付けない。
fn to_contents(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|m| {
            let role = if m.role == "assistant" { "model" } else { "user" };
            json!({ "role": role, "parts": [{ "text": m.content }] })
        })
        .collect()
}

fn extract_text(parsed: &GenerateResponse) -> String {
    parsed
        .candidates
        .first()
        .and_then(|c| c.content.as_ref())
        .map(|content| {
            content
                .parts
                .iter()
                .map(|p| p.text.as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

#[async_trait::async_trait]
impl LlmProvider for Gemini {
    fn id(&self) -> &'static str {
        "gemini"
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let body = json!({
            "systemInstruction": { "parts": [{ "text": req.system }] },
            "contents": to_contents(&req.messages),
            "generationConfig": {
                "maxOutputTokens": req.max_tokens + REASONING_HEADROOM,
                "temperature": req.temperature,
            },
        });

        let (status, text, latency_ms) = self.post(&req.model, body, REQUEST_TIMEOUT).await?;
        if status != 200 {
            return Err(classify_status(status, &text));
        }

        let parsed: GenerateResponse = serde_json::from_str(&text)
            .map_err(|e| LlmError::InvalidOutput(format!("JSON を解釈できない: {e}")))?;

        let output = extract_text(&parsed);
        if output.trim().is_empty() {
            // 思考に出力枠を使い切ると本文が空で返る。max_tokens 不足と
            // セーフティによる遮断を区別できるよう理由を載せる。
            let reason = parsed
                .candidates
                .first()
                .and_then(|c| c.finish_reason.clone())
                .unwrap_or_else(|| "不明".into());
            return Err(LlmError::InvalidOutput(format!(
                "応答が空（finishReason: {reason}）"
            )));
        }

        Ok(CompletionResponse {
            text: output,
            input_tokens: parsed.usage_metadata.as_ref().and_then(|u| u.prompt_token_count),
            output_tokens: parsed
                .usage_metadata
                .as_ref()
                .and_then(|u| u.candidates_token_count),
            latency_ms,
        })
    }

    /// 仕様書 7.5.5: `maxOutputTokens: 1` で 1 回送る。
    ///
    /// 本文が空で返っても構わない。ここで見たいのはキーが通るかどうかで、
    /// 生成できるかどうかではない。
    async fn verify(&self) -> Result<(), LlmError> {
        let body = json!({
            "contents": [{ "role": "user", "parts": [{ "text": "hi" }] }],
            "generationConfig": { "maxOutputTokens": 1 },
        });

        let (status, text, _) = self.post(&self.model, body, VERIFY_TIMEOUT).await?;
        if status == 200 {
            return Ok(());
        }
        Err(classify_status(status, &text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gemini は assistant を知らない。model に読み替えないと 400 になる。
    #[test]
    fn assistant_is_renamed_to_model() {
        let msgs = vec![
            ChatMessage::user("ごはん食べた？"),
            ChatMessage::assistant("食べたよー"),
        ];
        let contents = to_contents(&msgs);
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[1]["parts"][0]["text"], "食べたよー");
    }

    #[test]
    fn text_is_joined_across_parts() {
        let raw = r#"{
            "candidates": [{
                "content": {"parts": [{"text": "前半"}, {"text": "後半"}], "role": "model"},
                "finishReason": "STOP"
            }],
            "usageMetadata": {"promptTokenCount": 8, "candidatesTokenCount": 4}
        }"#;
        let parsed: GenerateResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(extract_text(&parsed), "前半後半");
        assert_eq!(
            parsed.usage_metadata.unwrap().candidates_token_count,
            Some(4)
        );
    }

    /// 思考で出力枠を使い切ると content ごと欠ける。panic せずに空を返すこと。
    #[test]
    fn a_candidate_without_content_is_not_a_panic() {
        let raw = r#"{"candidates": [{"finishReason": "MAX_TOKENS"}]}"#;
        let parsed: GenerateResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(extract_text(&parsed), "");
        assert_eq!(
            parsed.candidates[0].finish_reason.as_deref(),
            Some("MAX_TOKENS")
        );
    }

    #[test]
    fn an_empty_candidate_list_is_not_a_panic() {
        let parsed: GenerateResponse = serde_json::from_str(r#"{"candidates": []}"#).unwrap();
        assert_eq!(extract_text(&parsed), "");
    }
}
