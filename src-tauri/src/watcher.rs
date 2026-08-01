//! アプリ内で新着を監視して処理する（仕様書 6.1）。
//!
//! CLI の `watch` と同じ処理をアプリの常駐スレッドで回す。
//! 違いは、結果を通知で知らせるところ。材料不足で止まったことに
//! 気づけないと、結局そのまま無返信になる。

use std::time::Duration;

use momreply_core::{
    imessage::{self, watcher::Ticker},
    pipeline::{self, LengthPreset},
    store::Store,
};
use tauri::AppHandle;

use crate::notify;

/// 監視スレッドを起こす。
///
/// chat.db の接続はこのスレッドの中で作る。`Connection` はスレッドを
/// またげないため。
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        if let Err(why) = run(&app) {
            eprintln!("監視を開始できない: {why:#}");
        }
    });
}

fn run(app: &AppHandle) -> anyhow::Result<()> {
    let chat_db_path = imessage::default_path()?;
    let chat_db = imessage::open_readonly(&chat_db_path)?;
    let config = imessage::watcher::Config::default();
    let ticker = Ticker::new(&chat_db_path, config.clone())?;

    let mut last_poll: Option<i64> = None;

    loop {
        let now = chrono::Local::now().timestamp();
        let gap = imessage::gap_detected(last_poll, now, config.wake_gap_threshold);
        last_poll = Some(now);

        if let Err(why) = tick(app, &chat_db, gap) {
            // 1 回失敗しても監視は止めない。chat.db が一時的に
            // 読めないだけのこともある。
            eprintln!("監視の 1 周で失敗（継続）: {why:#}");
        }

        ticker.wait();
    }
}

fn tick(app: &AppHandle, chat_db: &rusqlite::Connection, gap: bool) -> anyhow::Result<()> {
    let store = Store::open_default()?;

    for target in store.list_targets()? {
        if !target.enabled {
            continue;
        }

        let runtime = store.target_runtime(target.id)?;
        let after = runtime.last_seen_rowid.unwrap_or(0);
        let new = imessage::messages_after(chat_db, &target.handles, after)?;
        if new.is_empty() {
            continue;
        }

        let plan = imessage::plan(new, gap);

        for (m, reason) in &plan.passed {
            if *reason == imessage::Passed::NotApplicable {
                continue;
            }
            store.record_processed(
                target.id,
                m.rowid,
                &m.chat_identifier,
                m.date.timestamp(),
                m.body.as_deref(),
                "skipped",
                Some(reason.label()),
                None,
                None,
                None,
            )?;
        }

        if let Some(message) = plan.actionable {
            let preset = LengthPreset::parse(&target.reply_preset).unwrap_or(LengthPreset::Mirror);
            let options = pipeline::Options {
                limits: pipeline::Limits::load(&store)?,
                preset,
                dry_run: dry_run(&store)?,
                redo_instruction: None,
                session_gap: Duration::from_secs(180 * 60),
            };

            let outcome = tauri::async_runtime::block_on(pipeline::process(
                chat_db, &store, &target, &message, &options,
            ))?;

            let who = &target.display_name;
            match outcome {
                pipeline::Outcome::NeedsAnswer(qs) => notify::needs_answer(app, who, &qs),
                pipeline::Outcome::Held(reason) => {
                    let draft = store.previous_draft(message.rowid)?.unwrap_or_default();
                    notify::awaiting_review(app, who, reason.label(), &draft);
                }
                pipeline::Outcome::Sent { .. } => {
                    let draft = store.previous_draft(message.rowid)?.unwrap_or_default();
                    notify::sent(app, who, &draft);
                }
                pipeline::Outcome::SentUnverified => notify::send_unverified(app, who),
                pipeline::Outcome::Failed(why) => notify::failed(app, who, &why),
                // スキップは通知しない。既返信やクールダウンは正常な動作で、
                // 毎回鳴ると通知そのものを見なくなる。
                pipeline::Outcome::Skipped(_) => {}
            }
        }

        if let Some(rowid) = plan.next_seen_rowid {
            store.set_last_seen_rowid(target.id, rowid)?;
        }
    }
    Ok(())
}

/// ドライラン。**既定は true**（仕様書 6.4.7）。
/// 明示的に false にしない限り送らない。
fn dry_run(store: &Store) -> anyhow::Result<bool> {
    Ok(store
        .get_kv("dry_run")?
        .map(|v| v != "false")
        .unwrap_or(true))
}
