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
/// **この名前が現在も有効かは疎通テストで確かめること。**
pub const DEFAULT_MODEL: &str = "gpt-5";

/// 疎通テストの出力枠。
///
/// 仕様書 7.5.5 は「`max_tokens: 1` で1回送る」としているが、
/// **推論モデルには通用しない。** 推論トークンを先に消費するため、
/// 枠が 1 だと出力に到達する前に必ず上限へ当たり、OpenAI はそれを
/// 200 の切り詰めではなく 400 で返す。キーが正しくても検証が
/// 失敗し続けることになる。
///
/// 推論ぶんを吸収できる程度に取る。1 回だけの呼び出しなので
/// この値でも費用は無視できる。
const VERIFY_MAX_TOKENS: u32 = 256;

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
    ) -> Result<(u16, String, u64), LlmError> {
        let client = http_client(timeout)?;

        let request = credentials::with_key(Provider::Openai, |key| {
            client
                .post(ENDPOINT)
                .header("authorization", format!("Bearer {key}"))
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

/// `temperature` を理由に 400 が返ったか。
fn rejects_temperature(body: &str) -> bool {
    body.contains("temperature")
        && (body.contains("unsupported_value") || body.contains("does not support"))
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
        // 新しめのモデルは max_tokens を受け付けず max_completion_tokens を要求する。
        let base = json!({
            "model": req.model,
            "messages": to_messages(&req.system, &req.messages),
            "max_completion_tokens": req.max_tokens + REASONING_HEADROOM,
        });

        let mut body = base.clone();
        body["temperature"] = json!(req.temperature);

        let (mut status, mut text, mut latency_ms) = self.post(body, REQUEST_TIMEOUT).await?;

        // 既定値以外の temperature を受け付けないモデルがある。
        // どのモデルが対応しているかを埋め込むと、モデルが増えるたびに
        // 古くなる。拒否されたら黙って外して 1 度だけやり直す。
        if status == 400 && rejects_temperature(&text) {
            let (s2, t2, l2) = self.post(base, REQUEST_TIMEOUT).await?;
            status = s2;
            text = t2;
            latency_ms = l2;
        }

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
            latency_ms,
        })
    }

    /// 仕様書 7.5.5: 最小の呼び出しを 1 回送る。
    ///
    /// temperature は載せない。モデルによっては既定値以外を拒否するため、
    /// キーの検証が温度設定のせいで落ちるのを避ける。
    /// 出力枠については [`VERIFY_MAX_TOKENS`] のコメントを参照。
    async fn verify(&self) -> Result<(), LlmError> {
        let body = json!({
            "model": self.model,
            "messages": [{ "role": "user", "content": "hi" }],
            "max_completion_tokens": VERIFY_MAX_TOKENS,
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

    /// temperature を拒否されたことを検出できること。
    /// 見落とすと、gpt-5 系で生成が常に失敗する。
    #[test]
    fn a_temperature_rejection_is_recognised() {
        let body = r#"{"error":{"message":"Unsupported value: 'temperature' does not support 0.8 with this model. Only the default (1) value is supported.","type":"invalid_request_error","param":"temperature","code":"unsupported_value"}}"#;
        assert!(rejects_temperature(body));
    }

    /// 別の理由の 400 で temperature を外して再送しても意味が無い。
    #[test]
    fn other_errors_do_not_look_like_a_temperature_rejection() {
        assert!(!rejects_temperature(
            r#"{"error":{"message":"model not found","code":"model_not_found"}}"#
        ));
        assert!(!rejects_temperature(""));
    }

    /// 仕様書 7.5.5 の字面（max_tokens: 1）に戻すと、推論モデルで
    /// 検証が必ず 400 になる。実機で踏んだので固定しておく。
    #[test]
    fn verify_budget_leaves_room_for_reasoning_tokens() {
        assert!(
            VERIFY_MAX_TOKENS >= 64,
            "疎通テストの出力枠が小さすぎる。推論トークンで使い切って 400 になる"
        );
    }
}
