//! UI ↔ Core の橋渡し（仕様書 7.5.3）。
//!
//! **キー本体を戻り値に含むコマンドを作らないこと。デバッグ目的でも作らない。**
//! ここで返してよいのは [`KeyStatus`] だけで、これは `masked`（末尾4文字）
//! しか持たない。core 側もキーを返す関数を公開していないため、
//! ここで誤って露出させようとしてもコンパイルが通らない。

use momreply_core::{
    llm::{self, credentials, KeyStatus, LlmError, Provider},
    store::Store,
};
use serde::Serialize;

fn parse_provider(provider: &str) -> Result<Provider, String> {
    Provider::parse(provider).ok_or_else(|| format!("不明なプロバイダ: {provider}"))
}

/// 設定中のモデル名を読む。未設定なら既定値。
fn model_for(provider: Provider) -> Option<String> {
    let store = Store::open_default().ok()?;
    store.get_kv(&provider.model_setting_key()).ok().flatten()
}

/// 疎通テストの結果を [`KeyStatus`] に反映する（仕様書 7.5.5）。
///
/// 401/403 でもキーは消さない。打ち間違いを直すときに再入力させないため。
/// ただし `verified: false` のままにして、自動送信の前提条件から外す。
async fn verify_into_status(provider: Provider, mut status: KeyStatus) -> KeyStatus {
    if !status.configured {
        return status;
    }

    let llm = match llm::build(provider, model_for(provider)) {
        Ok(llm) => llm,
        Err(why) => {
            status.error = Some(why.to_string());
            return status;
        }
    };

    match llm.verify().await {
        Ok(()) => {
            status.verified = true;
            status.last_verified_at = Some(now_unix());
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
            // モデル名が違う場合はここに来る。API の返答をそのまま見せないと
            // 何を直せばよいか分からない。キーは本文に載らない。
            status.verified = false;
            status.error = Some(format!("保存済み（未検証）: {}", brief(&other)));
        }
    }
    status
}

/// API の応答を UI に出せる長さに縮める。
fn brief(err: &LlmError) -> String {
    let msg = err.message();
    let head: String = msg.chars().take(200).collect();
    if head.trim().is_empty() {
        err.to_string()
    } else {
        head
    }
}

fn now_unix() -> i64 {
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

/// キーが 1 つも設定されていない間は自動送信を有効にできない（仕様書 7.5.4）。
#[tauri::command]
pub fn can_enable_auto_send() -> bool {
    Provider::with_keys()
        .into_iter()
        .any(credentials::is_configured)
}

// MARK: モデル設定

#[derive(Serialize)]
pub struct ModelSetting {
    provider: String,
    /// 実際に使う値。未設定なら既定値が入る。
    model: String,
    /// 既定値。UI のプレースホルダに使う。
    default_model: String,
    /// ユーザーが明示的に設定しているか。
    customized: bool,
}

#[tauri::command]
pub fn list_models() -> Result<Vec<ModelSetting>, String> {
    let store = Store::open_default().map_err(|e| e.to_string())?;
    Provider::with_keys()
        .into_iter()
        .map(|p| {
            let saved = store
                .get_kv(&p.model_setting_key())
                .map_err(|e| e.to_string())?;
            Ok(ModelSetting {
                provider: p.id().to_string(),
                model: saved
                    .clone()
                    .unwrap_or_else(|| p.default_model().to_string()),
                default_model: p.default_model().to_string(),
                customized: saved.is_some(),
            })
        })
        .collect()
}

/// モデル名を設定する。空文字を渡すと既定値に戻す。
#[tauri::command]
pub fn set_model(provider: String, model: String) -> Result<(), String> {
    let p = parse_provider(&provider)?;
    let store = Store::open_default().map_err(|e| e.to_string())?;
    store
        .set_kv(&p.model_setting_key(), model.trim())
        .map_err(|e| e.to_string())
}
