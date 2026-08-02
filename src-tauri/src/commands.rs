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

// MARK: 確認待ちの返信（仕様書 6.6）

#[derive(Serialize)]
pub struct PendingView {
    chat_rowid: i64,
    target_slug: String,
    display_name: String,
    received_at: i64,
    incoming: String,
    draft: String,
    status: String,
    reason: Option<String>,
}

fn open_chat_db() -> Result<rusqlite::Connection, String> {
    let path = momreply_core::imessage::default_path().map_err(|e| e.to_string())?;
    momreply_core::imessage::open_readonly(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_pending() -> Result<Vec<PendingView>, String> {
    let store = Store::open_default().map_err(|e| e.to_string())?;
    let items = store.pending_items(50).map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for item in items {
        out.push(PendingView {
            chat_rowid: item.chat_rowid,
            target_slug: item.target_slug,
            display_name: item.display_name,
            received_at: item.received_at,
            incoming: item.body.unwrap_or_default(),
            draft: item.draft.unwrap_or_default(),
            status: item.status,
            reason: item.skip_reason,
        });
    }
    Ok(out)
}

/// 人が確認して送る。**送信直前の既返信チェックは core 側で行う。**
///
/// 送信の検証は最大 30 秒かかる。chat.db の `Connection` は `Sync` でなく
/// `.await` をまたげないこともあり、丸ごと別スレッドに逃がす。
#[tauri::command]
pub async fn send_reply(chat_rowid: i64, text: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || send_reply_blocking(chat_rowid, &text))
        .await
        .map_err(|e| e.to_string())?
}

fn send_reply_blocking(chat_rowid: i64, text: &str) -> Result<String, String> {
    let store = Store::open_default().map_err(|e| e.to_string())?;
    let item = store
        .pending_items(50)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|i| i.chat_rowid == chat_rowid)
        .ok_or_else(|| format!("#{chat_rowid} は確認待ちにありません"))?;
    let target = store
        .target_by_slug(&item.target_slug)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("'{}' は登録されていません", item.target_slug))?;

    let chat_db = open_chat_db()?;
    let outcome = momreply_core::pipeline::run::send_manual(
        &chat_db,
        &store,
        &target,
        chat_rowid,
        &item.chat_guid,
        text,
    )
    .map_err(|e| e.to_string())?;

    Ok(match outcome {
        momreply_core::pipeline::Outcome::Sent { .. } => "送信しました".into(),
        momreply_core::pipeline::Outcome::SentUnverified => {
            "送信しましたが確認できませんでした。再送はしていません".into()
        }
        momreply_core::pipeline::Outcome::Skipped(_) => {
            "既に自分から返信済みだったため送信しませんでした".into()
        }
        momreply_core::pipeline::Outcome::Failed(why) => return Err(why),
        other => format!("{other:?}"),
    })
}

/// 追加指示つき再生成（仕様書 6.6 / 8.3）。
#[tauri::command]
pub async fn regenerate(
    chat_rowid: i64,
    instruction: Option<String>,
    length: Option<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        regenerate_blocking(chat_rowid, instruction, length)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn regenerate_blocking(
    chat_rowid: i64,
    instruction: Option<String>,
    length: Option<String>,
) -> Result<String, String> {
    use momreply_core::pipeline::{draft_reply, LengthPreset, Redo};

    let store = Store::open_default().map_err(|e| e.to_string())?;
    let item = store
        .pending_items(50)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|i| i.chat_rowid == chat_rowid)
        .ok_or_else(|| format!("#{chat_rowid} は確認待ちにありません"))?;
    let target = store
        .target_by_slug(&item.target_slug)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("'{}' は登録されていません", item.target_slug))?;

    let chat_db = open_chat_db()?;
    let message =
        momreply_core::imessage::reader::message_by_rowid(&chat_db, &target.handles, chat_rowid)
            .map_err(|e| e.to_string())?
            .ok_or("元のメッセージが見つかりません")?;

    let preset = LengthPreset::parse(length.as_deref().unwrap_or(&target.reply_preset))
        .ok_or("長さの指定が不正です")?;
    let instruction = instruction
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let draft = tauri::async_runtime::block_on(draft_reply(
        &chat_db,
        &store,
        &target,
        &message,
        preset,
        Some(Redo {
            instruction: instruction.as_deref(),
        }),
        momreply_core::pipeline::Urgency::Interactive,
    ))
    .map_err(|e| e.to_string())?;

    store
        .record_processed(
            target.id,
            chat_rowid,
            &item.chat_guid,
            item.received_at,
            item.body.as_deref(),
            &item.status,
            item.skip_reason.as_deref(),
            Some(&draft.text),
            Some(&draft.provider),
            Some(&draft.model),
        )
        .map_err(|e| e.to_string())?;

    Ok(draft.text)
}

/// この 1 件は返さない、と決める。連続カウンタは 0 に戻す（仕様書 6.4.5.1）。
#[tauri::command]
pub fn skip_pending(chat_rowid: i64) -> Result<(), String> {
    let store = Store::open_default().map_err(|e| e.to_string())?;
    if let Some(item) = store
        .pending_items(50)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|i| i.chat_rowid == chat_rowid)
    {
        let _ = store.reset_consecutive(item.target_id);
    }
    store
        .mark_skipped(chat_rowid, "user_skipped")
        .map_err(|e| e.to_string())
}

// MARK: キルスイッチとドライラン（仕様書 6.4.6 / 6.4.7）

#[derive(Serialize)]
pub struct RunMode {
    auto_send: bool,
    /// **既定は true。** 明示的に切らない限り送らない。
    dry_run: bool,
}

#[tauri::command]
pub fn get_run_mode() -> Result<RunMode, String> {
    let store = Store::open_default().map_err(|e| e.to_string())?;
    Ok(RunMode {
        auto_send: store
            .get_kv("auto_send_enabled")
            .map_err(|e| e.to_string())?
            .map(|v| v != "false")
            .unwrap_or(true),
        dry_run: store
            .get_kv("dry_run")
            .map_err(|e| e.to_string())?
            .map(|v| v != "false")
            .unwrap_or(true),
    })
}

#[tauri::command]
pub fn set_run_mode(auto_send: bool, dry_run: bool) -> Result<(), String> {
    let store = Store::open_default().map_err(|e| e.to_string())?;
    store
        .set_kv("auto_send_enabled", if auto_send { "true" } else { "false" })
        .map_err(|e| e.to_string())?;
    store
        .set_kv("dry_run", if dry_run { "true" } else { "false" })
        .map_err(|e| e.to_string())
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

// MARK: 相手ごとの設定

#[derive(Serialize)]
pub struct TargetView {
    slug: String,
    display_name: String,
    handles: Vec<String>,
    enabled: bool,
    auto_send: bool,
    /// プリセット名か `chars:400`。
    reply_preset: String,
    /// 文体の手本の数。0 だとその人らしさが出ない。
    fewshot_count: usize,
}

#[tauri::command]
pub fn list_targets() -> Result<Vec<TargetView>, String> {
    let store = Store::open_default().map_err(|e| e.to_string())?;
    Ok(store
        .list_targets()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|t| TargetView {
            fewshot_count: store.fewshot(t.id).map(|v| v.len()).unwrap_or(0),
            slug: t.slug,
            display_name: t.display_name,
            handles: t.handles,
            enabled: t.enabled,
            auto_send: t.auto_send,
            reply_preset: t.reply_preset,
        })
        .collect())
}

#[derive(Serialize)]
pub struct ChatChoice {
    chat_identifier: String,
    service: String,
    display_name: String,
    message_count: i64,
    last_message: Option<String>,
    /// 既に登録済みか。
    registered: bool,
}

/// 会話相手の候補を出す。**本文は読まない**（仕様書 10.2-3）。
#[tauri::command]
pub fn list_chat_choices(limit: u32) -> Result<Vec<ChatChoice>, String> {
    let chat_db = open_chat_db()?;
    let store = Store::open_default().map_err(|e| e.to_string())?;
    let taken: Vec<String> = store
        .list_targets()
        .map_err(|e| e.to_string())?
        .into_iter()
        .flat_map(|t| t.handles)
        .collect();

    Ok(momreply_core::imessage::list_chats(&chat_db, limit)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|c| ChatChoice {
            registered: taken.contains(&c.chat_identifier),
            chat_identifier: c.chat_identifier,
            service: c.service_name,
            display_name: c.display_name,
            message_count: c.message_count,
            last_message: c.last_message.map(|d| d.format("%Y-%m-%d").to_string()),
        })
        .collect())
}

/// 相手を登録する。
///
/// **登録した時点より前のメッセージは処理対象にならない**（仕様書 6.1）。
/// 保護は `Store::add_target` の内側にあり、ここから迂回できない。
#[tauri::command]
pub fn add_target(name: String, handles: Vec<String>) -> Result<String, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("表示名を入力してください".into());
    }
    if handles.is_empty() {
        return Err("会話を 1 つ以上選んでください".into());
    }

    let chat_db = open_chat_db()?;
    let mut store = Store::open_default().map_err(|e| e.to_string())?;
    let slug = unique_slug(&store, &handles[0]).map_err(|e| e.to_string())?;

    let target = store
        .add_target(
            &chat_db,
            momreply_core::store::NewTarget {
                slug: slug.clone(),
                display_name: name,
                handles,
            },
        )
        .map_err(|e| e.to_string())?;

    // 文体の手本をここで作る（仕様書 10.2-6）。
    // 無いまま生成すると、その人らしさが一切出ない。
    let pairs = build_fewshot(&chat_db, &store, &target).unwrap_or(0);

    Ok(format!(
        "{} を登録しました。ここより前のメッセージは処理されません。\n\
         文体の手本を {pairs} 組作りました。",
        target.display_name
    ))
}

/// 過去のやり取りから文体の手本を作る。
fn build_fewshot(
    chat_db: &rusqlite::Connection,
    store: &Store,
    target: &momreply_core::store::Target,
) -> anyhow::Result<usize> {
    momreply_core::fewshot::rebuild(chat_db, store, target.id, &target.handles, 40, 2000)
}

/// 直近の受信で返信案を作り、**確認待ちに積む**。
///
/// # なぜ処理位置の巻き戻しにしないか
///
/// 仕様書 6.1 の登録前バックログ保護は、過去の会話へ自動で返信が飛ぶのを
/// 防ぐためにある。`last_seen_rowid` を UI から巻き戻せるようにすると、
/// その保護を外す経路そのものができる。自動送信が有効なら、巻き戻した分の
/// 古いメッセージへ実際に返信が飛ぶ。
///
/// ここは違う。**最新の受信 1 件だけ**を、**必ず `awaiting_review` として**
/// 積む。ガードの判定を通らないので、押しても勝手には送信されない。
/// 送るかどうかは返信タブで人が決める（送信直前の既返信チェックは
/// `run::send_manual` の中にある）。
#[tauri::command]
pub async fn draft_latest(slug: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || draft_latest_blocking(&slug))
        .await
        .map_err(|e| e.to_string())?
}

fn draft_latest_blocking(slug: &str) -> Result<String, String> {
    use momreply_core::pipeline::{draft_reply, LengthPreset, Urgency};

    let chat_db = open_chat_db()?;
    let store = Store::open_default().map_err(|e| e.to_string())?;
    let target = store
        .target_by_slug(slug)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("'{slug}' は登録されていません"))?;

    let message = momreply_core::imessage::recent_messages(&chat_db, &target.handles, 50)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|m| !m.is_from_me && m.skip.is_none() && m.body.is_some())
        .next_back()
        .ok_or("返信できる受信メッセージが見つかりません")?;

    // 自分から返信済みのものに、あとから案を積まない（仕様書 6.4.3）。
    if momreply_core::imessage::count_own_replies_after(&chat_db, &target.handles, message.rowid)
        .map_err(|e| e.to_string())?
        > 0
    {
        return Err("この受信には既に自分から返信しています".into());
    }

    if store
        .pending_items(50)
        .map_err(|e| e.to_string())?
        .iter()
        .any(|i| i.chat_rowid == message.rowid)
    {
        return Err("この受信の返信案は既に確認待ちにあります".into());
    }

    let preset = LengthPreset::parse(&target.reply_preset).unwrap_or(LengthPreset::Mirror);
    let draft = tauri::async_runtime::block_on(draft_reply(
        &chat_db,
        &store,
        &target,
        &message,
        preset,
        None,
        Urgency::Interactive,
    ))
    .map_err(|e| e.to_string())?;

    if draft.text.trim().is_empty() {
        return Err("返信案が空でした".into());
    }

    store
        .record_processed(
            target.id,
            message.rowid,
            &message.chat_identifier,
            message.date.timestamp(),
            message.body.as_deref(),
            "awaiting_review",
            Some("manual"),
            Some(&draft.text),
            Some(&draft.provider),
            Some(&draft.model),
        )
        .map_err(|e| e.to_string())?;

    // 監視スレッドが同じ受信をもう一度拾わないよう処理位置を進める。
    // **進めるだけで、決して戻さない。**
    let runtime = store.target_runtime(target.id).map_err(|e| e.to_string())?;
    if runtime.last_seen_rowid.unwrap_or(0) < message.rowid {
        store
            .set_last_seen_rowid(target.id, message.rowid)
            .map_err(|e| e.to_string())?;
    }

    Ok("返信タブに入れました".into())
}

/// 文体の手本を作り直す。会話が増えたときに使う。
#[tauri::command]
pub fn rebuild_fewshot(slug: String) -> Result<usize, String> {
    let chat_db = open_chat_db()?;
    let store = Store::open_default().map_err(|e| e.to_string())?;
    let target = store
        .target_by_slug(&slug)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("'{slug}' は登録されていません"))?;
    build_fewshot(&chat_db, &store, &target).map_err(|e| e.to_string())
}

/// ハンドルから重複しない slug を作る。プロファイルのファイル名になる。
fn unique_slug(store: &Store, handle: &str) -> anyhow::Result<String> {
    let base: String = handle
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(24)
        .collect();
    let base = if base.is_empty() { "target".to_string() } else { base };

    for n in 0..100 {
        let candidate = if n == 0 { base.clone() } else { format!("{base}-{n}") };
        if store.target_by_slug(&candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    anyhow::bail!("slug を決められない")
}

/// 相手を削除する。**履歴・few-shot・質問もまとめて消える。**
#[tauri::command]
pub fn remove_target(slug: String) -> Result<(), String> {
    let store = Store::open_default().map_err(|e| e.to_string())?;
    let target = store
        .target_by_slug(&slug)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("'{slug}' は登録されていません"))?;
    store.remove_target(target.id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_target(
    slug: String,
    auto_send: Option<bool>,
    reply_preset: Option<String>,
) -> Result<(), String> {
    let store = Store::open_default().map_err(|e| e.to_string())?;
    let target = store
        .target_by_slug(&slug)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("'{slug}' は登録されていません"))?;

    if let Some(v) = auto_send {
        // キーが無い状態で自動送信を有効にすると、次の受信で生成が
        // 失敗して初めて気づくことになる。
        if v && !Provider::with_keys().into_iter().any(credentials::is_configured) {
            return Err("APIキーが設定されていません".into());
        }
        store.set_auto_send(target.id, v).map_err(|e| e.to_string())?;
    }
    if let Some(p) = reply_preset {
        momreply_core::pipeline::LengthPreset::parse(&p)
            .ok_or_else(|| format!("不明な長さ: {p}"))?;
        store.set_reply_preset(target.id, &p).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// MARK: 上限（仕様書 6.4.5）

#[derive(Serialize)]
pub struct LimitsView {
    max_consecutive_auto: u32,
    max_per_hour: u32,
    max_per_day: u32,
    stale_threshold_minutes: u32,
    monthly_soft_limit_usd: f64,
    monthly_hard_limit_usd: f64,
}

#[tauri::command]
pub fn get_limits() -> Result<LimitsView, String> {
    let store = Store::open_default().map_err(|e| e.to_string())?;
    let l = momreply_core::pipeline::Limits::load(&store).map_err(|e| e.to_string())?;
    Ok(LimitsView {
        max_consecutive_auto: l.max_consecutive_auto,
        max_per_hour: l.max_per_hour,
        max_per_day: l.max_per_day,
        stale_threshold_minutes: (l.stale_threshold.as_secs() / 60) as u32,
        monthly_soft_limit_usd: l.monthly_soft_limit_usd,
        monthly_hard_limit_usd: l.monthly_hard_limit_usd,
    })
}

#[tauri::command]
pub fn set_limit(key: String, value: f64) -> Result<(), String> {
    // 想定外のキーで kv を汚さない。
    const ALLOWED: [&str; 6] = [
        "max_consecutive_auto",
        "max_per_hour",
        "max_per_day",
        "stale_threshold_minutes",
        "monthly_soft_limit_usd",
        "monthly_hard_limit_usd",
    ];
    if !ALLOWED.contains(&key.as_str()) {
        return Err(format!("不明な設定: {key}"));
    }
    if value < 0.0 {
        return Err("負の値は設定できません".into());
    }

    let store = Store::open_default().map_err(|e| e.to_string())?;
    store
        .set_kv(&format!("limits.{key}"), &value.to_string())
        .map_err(|e| e.to_string())
}

// MARK: 生成に使うプロバイダ（仕様書 7.4 の llm.primary）

#[derive(Serialize)]
pub struct ProviderChoice {
    id: String,
    label: String,
    /// キーが保存されているか。Apple はキー不要。
    configured: bool,
    /// 疎通テストに通っているか。
    verified: bool,
    /// 実装済みか。未実装なら選べない。
    implemented: bool,
    /// 選べない理由。UI に出す。
    unavailable_reason: Option<String>,
}

fn label_of(p: Provider) -> &'static str {
    match p {
        Provider::Anthropic => "Anthropic (Claude)",
        Provider::Gemini => "Gemini",
        Provider::Openai => "OpenAI (ChatGPT)",
        Provider::Apple => "Apple Intelligence",
    }
}

#[tauri::command]
pub fn list_providers() -> Result<Vec<ProviderChoice>, String> {
    let all = [
        Provider::Anthropic,
        Provider::Gemini,
        Provider::Openai,
        Provider::Apple,
    ];

    Ok(all
        .into_iter()
        .map(|p| {
            let implemented = p != Provider::Apple;
            let mut status = credentials::status(p);
            load_verification(&mut status, p);

            let configured = if p.needs_key() { status.configured } else { true };
            let reason = if !implemented {
                Some("未実装です".to_string())
            } else if !configured {
                Some("APIキーが未設定です".to_string())
            } else if !status.verified {
                Some("疎通テストが済んでいません".to_string())
            } else {
                None
            };

            ProviderChoice {
                id: p.id().to_string(),
                label: label_of(p).to_string(),
                configured,
                verified: status.verified,
                implemented,
                unavailable_reason: reason,
            }
        })
        .collect())
}

/// 現在の主プロバイダ。未設定なら、使える中から自動で選ぶ。
#[tauri::command]
pub fn get_primary_provider() -> Result<String, String> {
    let store = Store::open_default().map_err(|e| e.to_string())?;
    if let Some(id) = store.get_kv("llm.primary").map_err(|e| e.to_string())? {
        if Provider::parse(&id).is_some() {
            return Ok(id);
        }
    }
    Ok(Provider::with_keys()
        .into_iter()
        .find(|p| credentials::is_configured(*p))
        .map(|p| p.id().to_string())
        .unwrap_or_default())
}

/// 主プロバイダを選ぶ。
///
/// **使えないものは選ばせない。** 選べてしまうと、次の受信で生成が
/// 失敗して初めて気づくことになる。
#[tauri::command]
pub fn set_primary_provider(provider: String) -> Result<(), String> {
    let p = parse_provider(&provider)?;
    if p == Provider::Apple {
        return Err("Apple Intelligence はまだ実装されていません".into());
    }
    if !credentials::is_configured(p) {
        return Err(format!("{} のAPIキーが設定されていません", label_of(p)));
    }
    let store = Store::open_default().map_err(|e| e.to_string())?;
    store
        .set_kv("llm.primary", p.id())
        .map_err(|e| e.to_string())
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
