//! UI ↔ Core の橋渡し（仕様書 7.5.3）。
//!
//! **キー本体を戻り値に含むコマンドを作らないこと。デバッグ目的でも作らない。**
//! ここで返してよいのは [`KeyStatus`] だけで、これは `masked`（末尾4文字）
//! しか持たない。core 側もキーを返す関数を公開していないため、
//! ここで誤って露出させようとしてもコンパイルが通らない。

use momreply_core::llm::{
    anthropic::Anthropic, credentials, KeyStatus, LlmError, LlmProvider, Provider,
};

fn parse_provider(provider: &str) -> Result<Provider, String> {
    Provider::parse(provider).ok_or_else(|| format!("不明なプロバイダ: {provider}"))
}

fn provider_impl(provider: Provider, model: Option<String>) -> Result<Box<dyn LlmProvider>, String> {
    match provider {
        Provider::Anthropic => Ok(Box::new(Anthropic::new(
            model.unwrap_or_else(|| momreply_core::llm::anthropic::DEFAULT_MODEL.to_string()),
        ))),
        // Gemini / OpenAI / Apple は未実装。
        other => Err(format!("{} はまだ実装されていません", other.id())),
    }
}

/// 疎通テストの結果を [`KeyStatus`] に反映する（仕様書 7.5.5）。
///
/// 401/403 でもキーは消さない。打ち間違いを直すときに再入力させないため。
/// ただし `verified: false` のままにして、自動送信の前提条件から外す。
async fn verify_into_status(provider: Provider, mut status: KeyStatus) -> KeyStatus {
    if !status.configured {
        return status;
    }

    let llm = match provider_impl(provider, None) {
        Ok(llm) => llm,
        Err(why) => {
            status.error = Some(why);
            return status;
        }
    };

    match llm.verify().await {
        Ok(()) => {
            status.verified = true;
            status.last_verified_at = Some(chrono_now());
            status.error = None;
        }
        Err(LlmError::Auth(_)) => {
            status.verified = false;
            status.error = Some("キーが正しくありません".into());
        }
        Err(LlmError::RateLimit(_)) => {
            status.verified = false;
            status.error = Some("保存済み（レート制限のため未検証）".into());
        }
        Err(other) => {
            status.verified = false;
            status.error = Some(format!("保存済み（未検証）: {other}"));
        }
    }
    status
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[tauri::command]
pub async fn set_api_key(provider: String, key: String) -> Result<KeyStatus, String> {
    let p = parse_provider(&provider)?;
    credentials::set(p, &key).map_err(|e| e.to_string())?;
    let status = credentials::status(p);
    Ok(verify_into_status(p, status).await)
}

#[tauri::command]
pub fn get_key_status(provider: String) -> Result<KeyStatus, String> {
    let p = parse_provider(&provider)?;
    Ok(credentials::status(p))
}

#[tauri::command]
pub fn list_key_statuses() -> Vec<KeyStatus> {
    Provider::with_keys()
        .into_iter()
        .map(credentials::status)
        .collect()
}

#[tauri::command]
pub fn delete_api_key(provider: String) -> Result<(), String> {
    let p = parse_provider(&provider)?;
    credentials::delete(p).map_err(|e| e.to_string())
}

/// 保存済みキーで再テストする。**キーを受け取らない。**
#[tauri::command]
pub async fn verify_api_key(provider: String) -> Result<KeyStatus, String> {
    let p = parse_provider(&provider)?;
    let status = credentials::status(p);
    if !status.configured {
        return Err("キーが設定されていません".into());
    }
    Ok(verify_into_status(p, status).await)
}

/// キーが 1 つも検証できていない間は自動送信を有効にできない（仕様書 7.5.4）。
#[tauri::command]
pub fn can_enable_auto_send() -> bool {
    Provider::with_keys()
        .into_iter()
        .any(credentials::is_configured)
}
