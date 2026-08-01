//! LLM プロバイダの抽象化（仕様書 7.1）。

pub mod anthropic;
pub mod credentials;
pub mod gemini;
pub mod openai;

use std::time::Duration;

use serde::{Deserialize, Serialize};

pub use credentials::KeyStatus;

/// 対応プロバイダ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Anthropic,
    Gemini,
    Openai,
    /// オンデバイス。API キー不要（仕様書 7.3）。
    Apple,
}

impl Provider {
    pub fn id(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::Gemini => "gemini",
            Provider::Openai => "openai",
            Provider::Apple => "apple",
        }
    }

    /// Keychain のアカウント名（仕様書 7.5.1）。
    pub fn keychain_account(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic_api_key",
            Provider::Gemini => "gemini_api_key",
            Provider::Openai => "openai_api_key",
            Provider::Apple => "apple_unused",
        }
    }

    /// API キーが要るか。
    pub fn needs_key(self) -> bool {
        !matches!(self, Provider::Apple)
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "anthropic" => Some(Provider::Anthropic),
            "gemini" => Some(Provider::Gemini),
            "openai" => Some(Provider::Openai),
            "apple" => Some(Provider::Apple),
            _ => None,
        }
    }

    /// キーを要するプロバイダの一覧。設定画面はこの順で並べる。
    pub fn with_keys() -> [Provider; 3] {
        [Provider::Anthropic, Provider::Gemini, Provider::Openai]
    }

    /// 既定モデル。
    ///
    /// **モデル名はハードコードせず設定で上書きできること**（仕様書 7.2）。
    /// ここの値はあくまで初期値で、実行時点で有効かは疎通テストで分かる。
    pub fn default_model(self) -> &'static str {
        match self {
            Provider::Anthropic => anthropic::DEFAULT_MODEL,
            Provider::Gemini => gemini::DEFAULT_MODEL,
            Provider::Openai => openai::DEFAULT_MODEL,
            Provider::Apple => "on-device",
        }
    }

    /// app.db の kv に入れるモデル設定のキー。
    pub fn model_setting_key(self) -> String {
        format!("model.{}", self.id())
    }
}

/// プロバイダ実装を作る。
pub fn build(provider: Provider, model: Option<String>) -> Result<Box<dyn LlmProvider>, LlmError> {
    let model = model
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| provider.default_model().to_string());

    match provider {
        Provider::Anthropic => Ok(Box::new(anthropic::Anthropic::new(model))),
        Provider::Gemini => Ok(Box::new(gemini::Gemini::new(model))),
        Provider::Openai => Ok(Box::new(openai::Openai::new(model))),
        Provider::Apple => Err(LlmError::InvalidOutput(
            "Apple Intelligence はまだ実装されていません".into(),
        )),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// `user` または `assistant`。
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        ChatMessage {
            role: "user".into(),
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        ChatMessage {
            role: "assistant".into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: u32,
    pub temperature: f32,
}

#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub text: String,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub latency_ms: u64,
}

/// 仕様書 7.1 のエラー分類。リトライ方針がここで決まる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmError {
    /// APIキー不正 → 通知してリトライしない。
    Auth(String),
    /// → バックオフしてリトライ。
    RateLimit(String),
    /// → リトライ。
    Network(String),
    /// → リトライ。
    Server(String),
    /// → リトライしない。
    InvalidOutput(String),
}

impl LlmError {
    /// リトライしてよいか。
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            LlmError::RateLimit(_) | LlmError::Network(_) | LlmError::Server(_)
        )
    }

    pub fn message(&self) -> &str {
        match self {
            LlmError::Auth(m)
            | LlmError::RateLimit(m)
            | LlmError::Network(m)
            | LlmError::Server(m)
            | LlmError::InvalidOutput(m) => m,
        }
    }
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Auth(_) => write!(f, "APIキーが正しくありません"),
            LlmError::RateLimit(_) => write!(f, "レート制限に達しました"),
            LlmError::Network(_) => write!(f, "ネットワークエラー: {}", self.message()),
            LlmError::Server(_) => write!(f, "サーバーエラー: {}", self.message()),
            LlmError::InvalidOutput(_) => write!(f, "応答を解釈できません: {}", self.message()),
        }
    }
}

impl std::error::Error for LlmError {}

/// HTTP ステータスから分類する。全プロバイダ共通。
pub fn classify_status(status: u16, body: &str) -> LlmError {
    // 本文はそのままエラーに載る。**リクエストヘッダは絶対に載せない**
    // （`x-api-key` が乗るため。仕様書 7.5.6）。
    let brief: String = body.chars().take(300).collect();
    match status {
        401 | 403 => LlmError::Auth(brief),
        429 => LlmError::RateLimit(brief),
        500..=599 => LlmError::Server(brief),
        _ => LlmError::InvalidOutput(format!("HTTP {status}: {brief}")),
    }
}

#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    fn id(&self) -> &'static str;

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError>;

    /// 疎通テスト（仕様書 7.5.5）。**最も安価な呼び出しを1回**行う。
    async fn verify(&self) -> Result<(), LlmError>;
}

/// 疎通テストのタイムアウト。UI を待たせすぎない。
pub const VERIFY_TIMEOUT: Duration = Duration::from_secs(20);

pub fn http_client(timeout: Duration) -> Result<reqwest::Client, LlmError> {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| LlmError::Network(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_policy_matches_the_spec() {
        assert!(!LlmError::Auth(String::new()).is_retryable());
        assert!(!LlmError::InvalidOutput(String::new()).is_retryable());
        assert!(LlmError::RateLimit(String::new()).is_retryable());
        assert!(LlmError::Network(String::new()).is_retryable());
        assert!(LlmError::Server(String::new()).is_retryable());
    }

    #[test]
    fn status_codes_map_to_the_right_class() {
        assert!(matches!(classify_status(401, ""), LlmError::Auth(_)));
        assert!(matches!(classify_status(403, ""), LlmError::Auth(_)));
        assert!(matches!(classify_status(429, ""), LlmError::RateLimit(_)));
        assert!(matches!(classify_status(500, ""), LlmError::Server(_)));
        assert!(matches!(classify_status(503, ""), LlmError::Server(_)));
        assert!(matches!(classify_status(400, ""), LlmError::InvalidOutput(_)));
    }

    /// エラー表示にキーが混ざらないこと。
    #[test]
    fn auth_error_display_carries_no_body() {
        let e = LlmError::Auth("x-api-key: sk-ant-SECRET".into());
        assert!(!e.to_string().contains("SECRET"));
    }

    #[test]
    fn only_apple_works_without_a_key() {
        assert!(Provider::Anthropic.needs_key());
        assert!(Provider::Gemini.needs_key());
        assert!(Provider::Openai.needs_key());
        assert!(!Provider::Apple.needs_key());
    }
}
