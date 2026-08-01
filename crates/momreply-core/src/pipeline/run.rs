//! 受信 1 件を最後まで処理する（仕様書 6.2）。
//!
//! 検知 → ガード → 生成 → **送信直前の再チェック** → 送信 → 検証 → 記録。
//!
//! # 二度チェックする理由
//!
//! 生成には数秒かかる。その間に iPhone で手で返信されることも、
//! 相手がメッセージを取り消すこともある（仕様書 6.4.3 / 5.1.1）。
//! 生成前の判定だけでは、その数秒の隙間で二重返信や、
//! 取り消された話への返信が起きる。

use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Local, TimeZone};
use rusqlite::Connection;

use crate::{
    imessage::{self, sender},
    pipeline::{draft_reply, guards, LengthPreset, Redo},
    store::{Store, Target},
};

/// 1 件を処理した結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// 送信して chat.db で確認できた。
    Sent { rowid: i64 },
    /// 送信したが確認できなかった。**再送しない。**
    SentUnverified,
    /// 生成したが送らず、確認に回した。
    Held(guards::HoldReason),
    /// 生成もしなかった。
    Skipped(guards::SkipReason),
    /// 答える材料が無い質問があるため人に聞く。
    NeedsAnswer(Vec<String>),
    /// 生成に失敗した。**何も送らない**（仕様書 1.2-3「失敗時は沈黙する」）。
    Failed(String),
}

pub struct Options {
    pub limits: guards::Limits,
    pub preset: LengthPreset,
    pub dry_run: bool,
    pub redo_instruction: Option<String>,
    pub session_gap: Duration,
}

/// 受信 1 件を処理する。
pub async fn process(
    chat_db: &Connection,
    store: &Store,
    target: &Target,
    message: &imessage::Message,
    options: &Options,
) -> Result<Outcome> {
    let now = Local::now();
    let runtime = store.target_runtime(target.id)?;

    // 連続カウンタのリセット判定を先に行う。
    // 日付が変わったときを取りこぼすと、翌朝の最初のメッセージが
    // 確認モードのまま放置される（仕様書 14.10）。
    let last_sent = runtime.last_sent_at.and_then(to_local);
    if guards::should_reset_consecutive(last_sent, now, options.session_gap) {
        store.reset_consecutive(target.id)?;
    }
    let runtime = store.target_runtime(target.id)?;

    let own_replies =
        imessage::count_own_replies_after(chat_db, &target.handles, message.rowid)?;

    let state = guards::State {
        now,
        received_at: message.date,
        own_replies_after: own_replies,
        last_sent_at: last_sent,
        sent_last_hour: store.sent_within(target.id, 3600)?,
        sent_last_day: store.sent_within(target.id, 86_400)?,
        consecutive_auto: runtime.consecutive_auto,
        month_cost_usd: store.month_cost_usd(target.id)?,
        auto_send_enabled: target.auto_send && kill_switch_on(store)?,
        dry_run: options.dry_run,
        escalated: false,
    };

    let verdict = guards::evaluate(&state, &options.limits);

    if let guards::Verdict::Skip(reason) = &verdict {
        record(store, target, message, "skipped", Some(reason.label()), None)?;
        return Ok(Outcome::Skipped(reason.clone()));
    }

    // 生成する。
    let redo = options.redo_instruction.as_deref().map(|i| Redo {
        instruction: Some(i),
    });
    let draft = match draft_reply(chat_db, store, target, message, options.preset, redo).await {
        Ok(d) => d,
        Err(why) => {
            // 失敗しても定型文などは送らない（仕様書 1.2-3）。
            record(store, target, message, "failed", Some("generation_failed"), None)?;
            return Ok(Outcome::Failed(why.to_string()));
        }
    };

    if let Some(reason) = draft.skipped {
        record(store, target, message, "skipped", Some(reason.label()), None)?;
        return Ok(Outcome::Skipped(reason));
    }

    if !draft.unanswerable.is_empty() {
        record(
            store,
            target,
            message,
            "awaiting_review",
            Some("needs_answer"),
            None,
        )?;
        return Ok(Outcome::NeedsAnswer(draft.unanswerable));
    }

    // 長さの暴走は送らずに確認へ（仕様書 6.2.1-5）。
    let hold = if draft.held_for_review {
        Some(guards::HoldReason::TooLong)
    } else if let guards::Verdict::Review(reason) = verdict {
        Some(reason)
    } else {
        None
    };

    if let Some(reason) = hold {
        let status = if reason == guards::HoldReason::DryRun {
            "dry_run"
        } else {
            "awaiting_review"
        };
        record(
            store,
            target,
            message,
            status,
            Some(reason.label()),
            Some(&draft),
        )?;
        return Ok(Outcome::Held(reason));
    }

    // ここから送信。まず直前の再チェック。
    record(store, target, message, "generating", None, Some(&draft))?;

    if imessage::count_own_replies_after(chat_db, &target.handles, message.rowid)? > 0 {
        // 生成している間に手で返信された。
        record(
            store,
            target,
            message,
            "skipped",
            Some(guards::SkipReason::AlreadyReplied.label()),
            Some(&draft),
        )?;
        return Ok(Outcome::Skipped(guards::SkipReason::AlreadyReplied));
    }

    // 生成中に相手が取り消していないか（仕様書 5.1.1）。
    if let Some(fresh) = imessage::reader::message_by_rowid(chat_db, &target.handles, message.rowid)?
    {
        if fresh.skip == Some(imessage::SkipReason::Retracted) {
            record(store, target, message, "skipped", Some("retracted"), Some(&draft))?;
            return Ok(Outcome::Skipped(guards::SkipReason::AlreadyReplied));
        }
    }

    // 受信元と同じ chat_identifier に返す（仕様書 6.3）。
    let baseline = imessage::max_rowid(chat_db, &target.handles)?.unwrap_or(0);
    if let Err(why) = sender::send(&message.chat_identifier, &draft.text) {
        store.mark_failed(message.rowid, "send_failed")?;
        return Ok(Outcome::Failed(why.to_string()));
    }

    match sender::verify(
        chat_db,
        &target.handles,
        baseline,
        &draft.text,
        sender::verify_timeout(),
    )? {
        sender::Outcome::Sent { rowid } => {
            store.mark_sent(message.rowid, &draft.text, Some(rowid))?;
            store.note_auto_sent(target.id, Local::now().timestamp())?;
            Ok(Outcome::Sent { rowid })
        }
        sender::Outcome::Unverified => {
            // 送れたかどうか分からない。**再送しない**（仕様書 6.3）。
            store.mark_failed(message.rowid, "verify_timeout")?;
            Ok(Outcome::SentUnverified)
        }
    }
}

/// 人が確認して送る（仕様書 6.6）。
///
/// 自動送信と違い、ガードのほとんどは通さない。人が見て決めたものを
/// レートリミットで止めるのは筋が通らない。
/// **ただし既返信チェックだけは必ず行う。** 画面を開いたまま放置して
/// いる間に iPhone で返信していることがあり、それを送ると二重返信になる。
pub fn send_manual(
    chat_db: &Connection,
    store: &Store,
    target: &Target,
    chat_rowid: i64,
    chat_identifier: &str,
    text: &str,
) -> Result<Outcome> {
    if text.trim().is_empty() {
        return Ok(Outcome::Failed("本文が空です".into()));
    }

    if imessage::count_own_replies_after(chat_db, &target.handles, chat_rowid)? > 0 {
        store.record_processed(
            target.id,
            chat_rowid,
            chat_identifier,
            Local::now().timestamp(),
            None,
            "skipped",
            Some(guards::SkipReason::AlreadyReplied.label()),
            None,
            None,
            None,
        )?;
        return Ok(Outcome::Skipped(guards::SkipReason::AlreadyReplied));
    }

    let baseline = imessage::max_rowid(chat_db, &target.handles)?.unwrap_or(0);
    if let Err(why) = sender::send(chat_identifier, text) {
        store.mark_failed(chat_rowid, "send_failed")?;
        return Ok(Outcome::Failed(why.to_string()));
    }

    // 人が介入したので連続自動返信のカウンタは 0 に戻す（仕様書 6.4.5.1）。
    store.reset_consecutive(target.id)?;

    match sender::verify(
        chat_db,
        &target.handles,
        baseline,
        text,
        sender::verify_timeout(),
    )? {
        sender::Outcome::Sent { rowid } => {
            store.mark_sent(chat_rowid, text, Some(rowid))?;
            Ok(Outcome::Sent { rowid })
        }
        sender::Outcome::Unverified => {
            store.mark_failed(chat_rowid, "verify_timeout")?;
            Ok(Outcome::SentUnverified)
        }
    }
}

/// キルスイッチ（仕様書 6.4.6）。既定は ON（＝自動送信を許す）。
fn kill_switch_on(store: &Store) -> Result<bool> {
    Ok(store
        .get_kv("auto_send_enabled")?
        .map(|v| v != "false")
        .unwrap_or(true))
}

fn to_local(ts: i64) -> Option<DateTime<Local>> {
    Local.timestamp_opt(ts, 0).single()
}

fn record(
    store: &Store,
    target: &Target,
    message: &imessage::Message,
    status: &str,
    skip_reason: Option<&str>,
    draft: Option<&crate::pipeline::Draft>,
) -> Result<()> {
    store.record_processed(
        target.id,
        message.rowid,
        &message.chat_identifier,
        message.date.timestamp(),
        message.body.as_deref(),
        status,
        skip_reason,
        draft.map(|d| d.text.as_str()),
        draft.map(|d| d.provider.as_str()),
        draft.map(|d| d.model.as_str()),
    )
}
