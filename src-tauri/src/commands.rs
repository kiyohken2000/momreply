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

/// 検証結果の保存先。
///
/// `credentials::status()` は Keychain を見るだけなので、疎通テストに
/// 成功したかどうかは分からない。ここに残さないと、再起動のたびに
/// 全プロバイダが「未検証」に戻る。
fn verified_key(provider: Provider) -> String {
    format!("key_verified_at.{}", provider.id())
}

fn load_verification(status: &mut KeyStatus, provider: Provider) {
    if !status.configured {
        return;
    }
    let Ok(store) = Store::open_default() else { return };
    if let Ok(Some(at)) = store.get_kv(&verified_key(provider)) {
        if let Ok(ts) = at.parse::<i64>() {
            status.verified = true;
            status.last_verified_at = Some(ts);
        }
    }
}

/// 検証状態を書き換える。キーを差し替えたり消したりしたときは必ず消す。
fn save_verification(provider: Provider, at: Option<i64>) {
    let Ok(store) = Store::open_default() else { return };
    let key = verified_key(provider);
    let _ = match at {
        Some(ts) => store.set_kv(&key, &ts.to_string()),
        None => store.set_kv(&key, ""),
    };
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
            let at = now_unix();
            status.verified = true;
            status.last_verified_at = Some(at);
            status.error = None;
            // 次回以降の起動でも検証済みとして扱えるように残す。
            save_verification(provider, Some(at));
        }
        Err(LlmError::Auth(_)) => {
            status.verified = false;
            status.error = Some("キーが正しくありません".into());
            save_verification(provider, None);
        }
        Err(err @ LlmError::RateLimit(_)) => {
            // 429 は「投げすぎ」とは限らない。OpenAI は残高不足でも 429 に
            // insufficient_quota を載せて返す。本文を捨てると両者を区別できず、
            // 待てば直ると誤解させてしまう。
            status.verified = false;
            status.error = Some(format!("保存済み（未検証・429）: {}", brief(&err)));
            save_verification(provider, None);
        }
        Err(other) => {
            // モデル名が違う場合はここに来る。API の返答をそのまま見せないと
            // 何を直せばよいか分からない。キーは本文に載らない。
            status.verified = false;
            status.error = Some(format!("保存済み（未検証）: {}", brief(&other)));
            save_verification(provider, None);
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
    // 先に検証状態を落とす。新しいキーを入れた瞬間、前のキーの
    // 検証結果は無効になる。ここで消さないと、検証前に「検証済み」と
    // 表示される隙間ができる。
    save_verification(p, None);
    credentials::set(p, &key).map_err(|e| e.to_string())?;
    let status = credentials::status(p);
    Ok(verify_into_status(p, status).await)
}

#[tauri::command]
pub fn get_key_status(provider: String) -> Result<KeyStatus, String> {
    let p = parse_provider(&provider)?;
    let mut status = credentials::status(p);
    load_verification(&mut status, p);
    Ok(status)
}

#[tauri::command]
pub fn list_key_statuses() -> Vec<KeyStatus> {
    Provider::with_keys()
        .into_iter()
        .map(|p| {
            let mut status = credentials::status(p);
            load_verification(&mut status, p);
            status
        })
        .collect()
}

#[tauri::command]
pub fn delete_api_key(provider: String) -> Result<(), String> {
    let p = parse_provider(&provider)?;
    // 検証状態も一緒に消す。消し忘れると、次に別のキーを入れたときに
    // 未検証のまま「検証済み」と表示される。
    save_verification(p, None);
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

// MARK: self.md

/// `self.md` の全文を返す。無ければテンプレートを作って返す。
#[tauri::command]
pub fn get_self_profile() -> Result<String, String> {
    momreply_core::profile::read_self().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_self_profile(content: String) -> Result<(), String> {
    momreply_core::profile::write_self(&content).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn self_profile_path() -> Result<String, String> {
    momreply_core::paths::self_profile()
        .map(|p| p.display().to_string())
        .map_err(|e| e.to_string())
}

// MARK: self.md への追記候補

#[derive(Serialize)]
pub struct FactCandidateView {
    id: i64,
    section: String,
    content: String,
    confidence: String,
    /// 根拠。人が正しさを判断するために必ず見せる。
    evidence_ask: Option<String>,
    evidence_reply: Option<String>,
}

#[tauri::command]
pub fn list_fact_candidates() -> Result<Vec<FactCandidateView>, String> {
    let store = Store::open_default().map_err(|e| e.to_string())?;
    Ok(store
        .pending_facts()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|c| FactCandidateView {
            id: c.id,
            section: c.section,
            content: c.content,
            confidence: c.confidence,
            evidence_ask: c.evidence_ask,
            evidence_reply: c.evidence_reply,
        })
        .collect())
}

/// 承認して `self.md` に追記する。**ここを通らない限り反映しない。**
#[tauri::command]
pub fn approve_fact(id: i64) -> Result<String, String> {
    let store = Store::open_default().map_err(|e| e.to_string())?;
    let c = store
        .fact_candidate(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("候補 #{id} がありません"))?;
    momreply_core::profile::append_to_section(&c.section, &c.content)
        .map_err(|e| e.to_string())?;
    store.set_fact_status(id, "approved").map_err(|e| e.to_string())?;
    momreply_core::profile::read_self().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reject_fact(id: i64) -> Result<(), String> {
    let store = Store::open_default().map_err(|e| e.to_string())?;
    store.set_fact_status(id, "rejected").map_err(|e| e.to_string())
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
