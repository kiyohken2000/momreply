//! 検証・運用 CLI。Phase 2 で Tauri UI を作るまでの操作口を兼ねる。

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use momreply_core::{
    facts, fewshot, imessage,
    pipeline::{self, LengthPreset},
    profile,
    store::{NewTarget, Store},
};

#[derive(Parser)]
#[command(name = "momreply-cli", about = "MomReply の検証・管理 CLI")]
struct Cli {
    /// chat.db のパス。省略時は ~/Library/Messages/chat.db。
    /// コピーを指定する場合は chat.db-wal と chat.db-shm も同じ場所に置くこと。
    #[arg(long, global = true)]
    db: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 会話相手の一覧を出す（本文は読まない）。ターゲットを決めるのに使う。
    Chats {
        #[arg(long, default_value_t = 30)]
        limit: u32,
    },
    /// 指定ハンドルの直近メッセージを出す。
    Messages {
        /// 対象ハンドル。複数指定可。
        #[arg(long, required = true)]
        handle: Vec<String>,

        #[arg(long, default_value_t = 20)]
        limit: u32,

        /// タップバックなどの除外対象も表示する。
        #[arg(long)]
        include_skipped: bool,
    },
    /// 返信対象の相手を管理する。
    #[command(subcommand)]
    Target(TargetCmd),
    /// 連投がどこまで 1 通としてまとめられるかを見る。
    ///
    /// 生成も送信もしない。まとめ方の確認だけに使う。
    Burst {
        #[arg(long)]
        slug: String,
        /// 末尾にする ROWID。省略すると直近の受信。
        #[arg(long)]
        rowid: Option<i64>,
    },
    /// 新着を検知して処理し続ける（仕様書 6.1）。
    ///
    /// 既定はドライラン。実際に送るには --live を明示する。
    Watch {
        #[arg(long)]
        slug: String,
        /// **実際に送信する。** 指定しない限り送らない。
        #[arg(long)]
        live: bool,
        /// 1 回だけ確認して終了する。
        #[arg(long)]
        once: bool,
    },
    /// 送信経路の疎通確認。**自分のアカウント宛にしか送らない。**
    SendTest {
        /// 宛先。この Mac の iMessage 送信アカウントと一致する必要がある。
        #[arg(long)]
        to: String,
        #[arg(long, default_value = "MomReply の送信テストです")]
        text: String,
    },
    /// self.md への追記候補を扱う。**承認するまで反映しない。**
    #[command(subcommand)]
    Facts(FactCmd),
    /// 生成に使う設定を確認・変更する。
    Config {
        /// 主プロバイダ（anthropic|gemini|openai）。
        #[arg(long)]
        primary: Option<String>,
        /// プロンプトで名乗る自分の名前。
        #[arg(long)]
        user_name: Option<String>,
    },
    /// 文体の手本（few-shot）を作り直す。
    Fewshot {
        #[arg(long)]
        slug: String,
        /// 保持するペア数。
        #[arg(long, default_value_t = 40)]
        limit: usize,
        /// 遡るメッセージ件数。
        #[arg(long, default_value_t = 2000)]
        scan: u32,
    },
    /// 返信案を作る。**送信はしない**（Phase 1 はドライランのみ）。
    Generate {
        #[arg(long)]
        slug: String,
        /// 対象メッセージの ROWID。省略すると直近の受信メッセージを使う。
        #[arg(long)]
        rowid: Option<i64>,
        /// mirror|short|normal|long|very_long。
        /// 省略すると相手ごとの既定値（target set --preset）を使う。
        #[arg(long)]
        length: Option<String>,
        /// 前回の案が使えなかったときのやり直し。
        /// 値を渡すとその指示で書き直す。空文字なら同じ条件でやり直す。
        #[arg(long)]
        redo: Option<String>,
    },
}

#[derive(Subcommand)]
enum FactCmd {
    /// 過去のやり取りから候補を抽出する。
    ///
    /// **指定した会話しか読まない。** `--handle` は `chats` の
    /// CHAT_IDENTIFIER をそのまま渡す。複数指定可。
    Scan {
        #[arg(long, required = true)]
        handle: Vec<String>,
        #[arg(long, default_value_t = 1000)]
        scan: u32,
        /// 抽出に回すペア数の上限。費用と外部に出る量を抑える。
        #[arg(long, default_value_t = 60)]
        max_pairs: usize,
    },
    /// 未承認の候補を根拠つきで並べる。
    List,
    /// 候補を承認して self.md に追記する。
    Approve {
        #[arg(long)]
        id: i64,
    },
    /// 候補を却下する。
    Reject {
        #[arg(long)]
        id: i64,
    },
}

#[derive(Subcommand)]
enum TargetCmd {
    /// 相手を登録する。登録時点より前のメッセージは処理対象にならない。
    Add {
        /// 識別子。プロファイルのファイル名になる（例: mother）。
        #[arg(long)]
        slug: String,
        /// 表示名（例: 母）。
        #[arg(long)]
        name: String,
        /// chat_identifier。`chats` の出力から取る。複数指定可。
        #[arg(long, required = true)]
        handle: Vec<String>,
    },
    /// 登録済みの相手を一覧する。
    List,
    /// 相手を削除する。関連する履歴・few-shot もまとめて消える。
    Rm {
        #[arg(long)]
        slug: String,
    },
    /// 未処理の新着を確認する（生成はまだしない）。
    Pending {
        #[arg(long)]
        slug: String,
    },
    /// 相手ごとの設定を変える。
    Set {
        #[arg(long)]
        slug: String,
        /// 既定の長さ。プリセット（mirror|short|normal|long|very_long）か、
        /// 目標文字数を指定する chars:400 の形。
        #[arg(long)]
        preset: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let chat_db_path = match cli.db {
        Some(p) => p,
        None => imessage::default_path()?,
    };
    let chat_db = imessage::open_readonly(&chat_db_path)?;

    match cli.command {
        Command::Chats { limit } => cmd_chats(&chat_db, limit),
        Command::Messages {
            handle,
            limit,
            include_skipped,
        } => cmd_messages(&chat_db, &handle, limit, include_skipped),
        Command::Target(cmd) => cmd_target(&chat_db, cmd),
        Command::Burst { slug, rowid } => cmd_burst(&chat_db, &slug, rowid),
        Command::Watch { slug, live, once } => {
            cmd_watch(&chat_db, &chat_db_path, &slug, live, once).await
        }
        Command::SendTest { to, text } => cmd_send_test(&chat_db, &to, &text),
        Command::Facts(cmd) => cmd_facts(&chat_db, cmd).await,
        Command::Config { primary, user_name } => cmd_config(primary, user_name),
        Command::Fewshot { slug, limit, scan } => cmd_fewshot(&chat_db, &slug, limit, scan),
        Command::Generate {
            slug,
            rowid,
            length,
            redo,
        } => cmd_generate(&chat_db, &slug, rowid, length.as_deref(), redo).await,
    }
}

async fn cmd_facts(chat_db: &rusqlite::Connection, cmd: FactCmd) -> Result<()> {
    let store = Store::open_default()?;

    match cmd {
        FactCmd::Scan {
            handle,
            scan,
            max_pairs,
        } => {
            let known: Vec<String> = imessage::list_chats(chat_db, 10_000)?
                .into_iter()
                .map(|c| c.chat_identifier)
                .collect();
            for h in &handle {
                if !known.contains(h) {
                    bail!("chat.db に '{h}' の会話がありません");
                }
            }

            println!("走査する会話: {}", handle.join(", "));
            println!("これらの会話の「質問と自分の返信」が LLM に送られます。");
            println!();

            let report = facts::scan(chat_db, &store, &handle, scan, max_pairs).await?;
            println!("質問に答えたやり取り {} 件を検査（{} 回の呼び出し）",
                report.pairs_examined, report.batches);
            println!("新しい候補: {} 件", report.candidates_added);
            if report.candidates_added > 0 {
                println!();
                println!("確認する: ./scripts/cli.sh facts list");
            }
        }

        FactCmd::List => {
            let pending = store.pending_facts()?;
            if pending.is_empty() {
                println!("未承認の候補はありません。");
                return Ok(());
            }
            println!("未承認 {} 件（承認するまで self.md には反映されません）", pending.len());
            println!();
            for c in &pending {
                println!("#{:<4} [{}] {} / {}", c.id, c.confidence, c.section, c.content);
                if let (Some(ask), Some(reply)) = (&c.evidence_ask, &c.evidence_reply) {
                    println!("      根拠: 「{}」", ask.replace('\n', " "));
                    println!("          → 「{}」", reply.replace('\n', " "));
                }
                println!();
            }
            println!("承認: ./scripts/cli.sh facts approve --id <ID>");
            println!("却下: ./scripts/cli.sh facts reject --id <ID>");
        }

        FactCmd::Approve { id } => {
            let c = store
                .fact_candidate(id)?
                .with_context(|| format!("候補 #{id} がありません"))?;
            profile::append_to_section(&c.section, &c.content)?;
            store.set_fact_status(id, "approved")?;
            println!("self.md の「{}」に追記しました:", c.section);
            println!("  - {}", c.content);
        }

        FactCmd::Reject { id } => {
            store.set_fact_status(id, "rejected")?;
            println!("候補 #{id} を却下しました。");
        }
    }
    Ok(())
}

async fn cmd_watch(
    chat_db: &rusqlite::Connection,
    chat_db_path: &std::path::Path,
    slug: &str,
    live: bool,
    once: bool,
) -> Result<()> {
    let store = Store::open_default()?;
    let target = store
        .target_by_slug(slug)?
        .with_context(|| format!("'{slug}' は登録されていない"))?;
    let preset = LengthPreset::parse(&target.reply_preset)
        .with_context(|| format!("不明な長さ指定: {}", target.reply_preset))?;

    let options = pipeline::Options {
        limits: pipeline::Limits::load(&store)?,
        preset,
        // 明示しない限り送らない。
        dry_run: !live,
        redo_instruction: None,
        session_gap: std::time::Duration::from_secs(180 * 60),
    };

    println!("監視: {} ({})", target.display_name, target.slug);
    println!("  ハンドル: {}", target.handles.join(", "));
    println!("  長さ: {}", target.reply_preset);
    if live {
        println!("  **実送信モード**（--live）");
    } else {
        println!("  ドライラン（送信しません）");
    }
    println!();

    let config = imessage::watcher::Config::default();
    let ticker = imessage::watcher::Ticker::new(chat_db_path, config.clone())?;
    let mut last_poll: Option<i64> = None;

    loop {
        let now = chrono::Local::now().timestamp();
        let gap = imessage::gap_detected(last_poll, now, config.wake_gap_threshold);
        if gap {
            println!("[{}] 時間が空いたので、溜まった分は最新1件だけ処理します",
                chrono::Local::now().format("%H:%M:%S"));
        }
        last_poll = Some(now);

        let runtime = store.target_runtime(target.id)?;
        let after = runtime.last_seen_rowid.unwrap_or(0);
        let new = imessage::messages_after(chat_db, &target.handles, after)?;
        let plan = imessage::plan_with_burst(chat_db, &target.handles, new, gap)?;

        for (m, reason) in &plan.passed {
            if *reason != imessage::Passed::NotApplicable {
                let what = if *reason == imessage::Passed::Merged {
                    "連投としてまとめた"
                } else {
                    "見送り"
                };
                println!(
                    "  {what} #{} ({}) {}",
                    m.rowid,
                    reason.label(),
                    m.body.as_deref().unwrap_or("").replace('\n', " ")
                );
                store.record_processed(
                    target.id, m.rowid, &m.chat_identifier, m.date.timestamp(),
                    m.body.as_deref(), "skipped", Some(reason.label()), None, None, None,
                )?;
            }
        }

        if let Some(message) = plan.actionable {
            println!(
                "[{}] 新着 #{} {}",
                message.date.format("%H:%M:%S"),
                message.rowid,
                message.body.as_deref().unwrap_or("").replace('\n', " ")
            );
            match pipeline::process(chat_db, &store, &target, &message, &options).await? {
                pipeline::Outcome::Sent { rowid } => {
                    println!("  送信しました（chat.db ROWID {rowid}）")
                }
                pipeline::Outcome::SentUnverified => {
                    println!("  送信したが確認できませんでした。再送はしません")
                }
                pipeline::Outcome::Held(reason) => {
                    let draft = store.previous_draft(message.rowid)?.unwrap_or_default();
                    println!("  送信せず確認へ（{}）", reason.label());
                    println!("  返信案: {draft}");
                }
                pipeline::Outcome::Skipped(reason) => {
                    println!("  スキップ（{}）", reason.label())
                }
                pipeline::Outcome::Failed(why) => println!("  失敗: {why}"),
            }
        }

        if let Some(rowid) = plan.next_seen_rowid {
            store.set_last_seen_rowid(target.id, rowid)?;
        }

        if once {
            return Ok(());
        }
        match ticker.wait() {
            imessage::watcher::Tick::FileChanged => {}
            imessage::watcher::Tick::Interval => {}
        }
    }
}

/// この Mac の iMessage 送信アカウント一覧を chat.db から取る。
///
/// テスト送信の宛先を自分に限定するために使う。
fn own_accounts(chat_db: &rusqlite::Connection) -> Result<Vec<String>> {
    let mut stmt = chat_db.prepare(
        "SELECT DISTINCT account FROM message
         WHERE is_from_me = 1 AND account IS NOT NULL AND account != ''",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;

    let mut out = Vec::new();
    for row in rows {
        // `E:foo@example.com` / `P:+81...` の形で入っている。
        let raw = row?;
        let id = raw.split_once(':').map(|(_, rest)| rest).unwrap_or(&raw);
        if !id.trim().is_empty() {
            out.push(id.to_string());
        }
    }
    Ok(out)
}

fn cmd_send_test(chat_db: &rusqlite::Connection, to: &str, text: &str) -> Result<()> {
    // CLAUDE.md ルール2: テスト送信は自分の Apple ID 宛にのみ行う。
    // 打ち間違いで他人に飛ばさないよう、コード側で弾く。
    let accounts = own_accounts(chat_db)?;
    if !accounts.iter().any(|a| a == to) {
        bail!(
            "'{to}' はこの Mac の iMessage 送信アカウントではありません。\n\
             テスト送信は自分宛にのみ行います。使えるのは:\n  {}",
            accounts.join("\n  ")
        );
    }

    let handles = vec![to.to_string()];
    let baseline = imessage::max_rowid(chat_db, &handles)?.unwrap_or(0);

    println!("宛先: {to}（自分のアカウント）");
    println!("本文: {text}");
    println!("送信前の最大 ROWID: {baseline}");
    println!();

    imessage::sender::send(to, text)?;
    println!("osascript は成功。ただしこれは送信できた証拠にならないので chat.db で確認します。");

    let timeout = imessage::sender::verify_timeout();
    println!("検証中（最大 {} 秒）…", timeout.as_secs());

    match imessage::sender::verify(chat_db, &handles, baseline, text, timeout)? {
        imessage::sender::Outcome::Sent { rowid } => {
            println!();
            println!("送信を確認しました。chat.db の ROWID = {rowid}");
        }
        imessage::sender::Outcome::Unverified => {
            println!();
            println!("chat.db に現れませんでした。送信に失敗した可能性があります。");
            println!("**自動で再送はしません**（実は届いていた場合に二重送信になるため）。");
            println!("メッセージ.app を開いて実際の状態を確認してください。");
        }
    }
    Ok(())
}

fn cmd_config(primary: Option<String>, user_name: Option<String>) -> Result<()> {
    let store = Store::open_default()?;

    if let Some(p) = primary {
        momreply_core::llm::Provider::parse(&p)
            .with_context(|| format!("不明なプロバイダ: {p}"))?;
        store.set_kv("llm.primary", &p)?;
        println!("主プロバイダ: {p}");
    }
    if let Some(name) = user_name {
        store.set_kv("user_name", &name)?;
        println!("自分の名前: {name}");
    }

    println!();
    println!(
        "llm.primary = {}",
        store.get_kv("llm.primary")?.unwrap_or_else(|| "(未設定)".into())
    );
    println!(
        "user_name   = {}",
        store.get_kv("user_name")?.unwrap_or_else(|| "(未設定)".into())
    );
    for p in momreply_core::llm::Provider::with_keys() {
        println!(
            "model.{:<10} = {}",
            p.id(),
            store
                .get_kv(&p.model_setting_key())?
                .unwrap_or_else(|| format!("{} (既定)", p.default_model()))
        );
    }
    Ok(())
}

fn cmd_fewshot(
    chat_db: &rusqlite::Connection,
    slug: &str,
    limit: usize,
    scan: u32,
) -> Result<()> {
    let store = Store::open_default()?;
    let target = store
        .target_by_slug(slug)?
        .with_context(|| format!("'{slug}' は登録されていない"))?;

    let n = fewshot::rebuild(chat_db, &store, target.id, &target.handles, limit, scan)?;
    println!("{} 件のペアを保存しました（直近 {scan} 件を走査）", n);

    if n < 10 {
        println!();
        println!("警告: ペアが少ないため文体の再現度が落ちます。--scan を増やしてください。");
    }

    let pairs = store.fewshot(target.id)?;
    println!();
    println!("先頭 3 件:");
    for p in pairs.iter().take(3) {
        println!("  相手: {}", p.incoming.replace('\n', " "));
        println!("  自分: {}", p.reply.replace('\n', " "));
    }
    Ok(())
}

async fn cmd_generate(
    chat_db: &rusqlite::Connection,
    slug: &str,
    rowid: Option<i64>,
    length: Option<&str>,
    redo: Option<String>,
) -> Result<()> {
    let store = Store::open_default()?;
    let target = store
        .target_by_slug(slug)?
        .with_context(|| format!("'{slug}' は登録されていない"))?;

    let length = length.unwrap_or(&target.reply_preset);
    let preset = LengthPreset::parse(length)
        .with_context(|| format!("不明な長さ指定: {length}"))?;

    // 対象メッセージを決める。
    let recent = imessage::recent_messages(chat_db, &target.handles, 100)?;
    let message = match rowid {
        Some(id) => recent
            .into_iter()
            .find(|m| m.rowid == id)
            .with_context(|| format!("ROWID {id} が直近 100 件に無い"))?,
        None => recent
            .into_iter()
            .filter(|m| !m.is_from_me && m.skip.is_none())
            .next_back()
            .context("生成対象になる受信メッセージが見つからない")?,
    };

    println!("対象 #{} {}", message.rowid, message.date.format("%m-%d %H:%M"));

    let redo = redo.as_deref().map(|instruction| {
        let trimmed = instruction.trim();
        if trimmed.is_empty() {
            println!("やり直し（指示なし）");
        } else {
            println!("やり直し: {trimmed}");
        }
        println!();
        pipeline::Redo {
            instruction: (!trimmed.is_empty()).then_some(trimmed),
        }
    });

    let draft = pipeline::draft_reply(
        chat_db, &store, &target, &message, preset, redo, pipeline::Urgency::Interactive,
    )
    .await?;

    // 連投をまとめていれば、まとめた全文がこちらに入る。
    for line in draft.incoming.lines() {
        println!("  {line}");
    }
    println!();

    if let Some(reason) = &draft.skipped {
        println!("ガードにより生成しませんでした: {}", reason.label());
        if matches!(reason, pipeline::guards::SkipReason::AlreadyReplied) {
            println!("  このメッセージには既に自分から返信しています（二重返信の防止）。");
        }
        store.record_processed(
            target.id,
            message.rowid,
            &message.chat_identifier,
            message.date.timestamp(),
            message.body.as_deref(),
            "skipped",
            Some(reason.label()),
            None,
            None,
            None,
        )?;
        return Ok(());
    }

    println!("--- 返信案（送信していません） ---");
    println!("{}", draft.text);
    println!("---");
    println!(
        "{} / {} / {}ms / in {:?} out {:?}",
        draft.provider, draft.model, draft.latency_ms, draft.input_tokens, draft.output_tokens
    );
    if draft.held_for_review {
        println!("※ 長さの上限を超えたため、自動送信の対象から外れます");
    }
    println!();
    println!("この案が使えないときは、指示を付けてやり直せます:");
    println!(
        "  ./scripts/cli.sh generate --slug {slug} --rowid {} --redo \"月曜は来ないでほしいと伝えて\"",
        message.rowid
    );

    store.record_processed(
        target.id,
        message.rowid,
        &message.chat_identifier,
        message.date.timestamp(),
        Some(draft.incoming.as_str()),
        if draft.held_for_review { "awaiting_review" } else { "dry_run" },
        None,
        Some(&draft.text),
        Some(&draft.provider),
        Some(&draft.model),
    )?;
    Ok(())
}

/// 連投のまとめ方を確認する。LLM を呼ばない。
fn cmd_burst(chat_db: &rusqlite::Connection, slug: &str, rowid: Option<i64>) -> Result<()> {
    let store = Store::open_default()?;
    let target = store
        .target_by_slug(slug)?
        .with_context(|| format!("'{slug}' は登録されていない"))?;

    let recent = imessage::recent_messages(chat_db, &target.handles, 100)?;
    let last = match rowid {
        Some(id) => recent
            .iter()
            .find(|m| m.rowid == id)
            .with_context(|| format!("ROWID {id} が直近 100 件に無い"))?,
        None => recent
            .iter()
            .filter(|m| !m.is_from_me && m.skip.is_none())
            .next_back()
            .context("受信メッセージが見つからない")?,
    };

    let group = imessage::burst(chat_db, &target.handles, last, imessage::BURST_WINDOW)?;
    println!("末尾 #{} からまとめると {} 件:", last.rowid, group.len());
    for m in &group {
        println!("  #{} {}", m.rowid, m.date.format("%m-%d %H:%M:%S"));
    }
    println!();
    println!("--- 生成に渡る本文 ---");
    println!("{}", imessage::burst_text(&group));
    println!("---");
    Ok(())
}

fn cmd_chats(conn: &rusqlite::Connection, limit: u32) -> Result<()> {
    let chats = imessage::list_chats(conn, limit)?;
    if chats.is_empty() {
        println!("会話が見つからない。");
        return Ok(());
    }

    println!("{:<40} {:<10} {:>7}  {}", "CHAT_IDENTIFIER", "SERVICE", "件数", "最終");
    for c in &chats {
        let last = c
            .last_message
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "-".to_string());
        let name = if c.display_name.is_empty() {
            String::new()
        } else {
            format!("  ({})", c.display_name)
        };
        println!(
            "{:<40} {:<10} {:>7}  {}{}",
            c.chat_identifier, c.service_name, c.message_count, last, name
        );
    }
    Ok(())
}

fn cmd_messages(
    conn: &rusqlite::Connection,
    handles: &[String],
    limit: u32,
    include_skipped: bool,
) -> Result<()> {
    let messages = imessage::recent_messages(conn, handles, limit)?;
    if messages.is_empty() {
        println!("該当メッセージなし。--handle を `chats` の CHAT_IDENTIFIER と突き合わせること。");
        return Ok(());
    }

    let mut shown = 0usize;
    let mut skipped = 0usize;
    let mut from_text_column = 0usize;

    for m in &messages {
        if m.skip.is_some() {
            skipped += 1;
            if !include_skipped {
                continue;
            }
        }
        if m.body_from_text_column {
            from_text_column += 1;
        }
        shown += 1;

        let who = if m.is_from_me { "自分" } else { "相手" };
        let mut tags = Vec::new();
        if m.edited {
            tags.push("編集済".to_string());
        }
        if let Some(reason) = m.skip {
            tags.push(format!("除外:{}", reason.label()));
        }
        if m.body_from_text_column {
            tags.push("text列".to_string());
        }
        let tag = if tags.is_empty() {
            String::new()
        } else {
            format!("  [{}]", tags.join(" "))
        };

        println!(
            "#{:<8} {} {} {}{}",
            m.rowid,
            m.date.format("%Y-%m-%d %H:%M:%S"),
            who,
            m.chat_identifier,
            tag
        );
        match &m.body {
            Some(body) => {
                for line in body.lines() {
                    println!("    {line}");
                }
            }
            None => println!("    (本文なし)"),
        }
        println!();
    }

    println!("---");
    println!("取得 {} 件 / 表示 {} 件 / 除外 {} 件", messages.len(), shown, skipped);
    println!(
        "本文の取得元: attributedBody {} 件 / text 列フォールバック {} 件",
        messages.iter().filter(|m| m.body.is_some()).count() - from_text_column,
        from_text_column
    );
    Ok(())
}

fn cmd_target(chat_db: &rusqlite::Connection, cmd: TargetCmd) -> Result<()> {
    let mut store = Store::open_default()?;

    match cmd {
        TargetCmd::Add { slug, name, handle } => {
            // 存在しないハンドルを打ち間違いで登録すると、
            // 何も来ないまま動いているように見えてしまう。先に確かめる。
            let known: Vec<String> = imessage::list_chats(chat_db, 10_000)?
                .into_iter()
                .map(|c| c.chat_identifier)
                .collect();
            for h in &handle {
                if !known.contains(h) {
                    bail!(
                        "chat.db に '{h}' の会話が無い。`momreply-cli chats` の \
                         CHAT_IDENTIFIER をそのまま使うこと"
                    );
                }
            }

            let target = store
                .add_target(
                    chat_db,
                    NewTarget {
                        slug: slug.clone(),
                        display_name: name,
                        handles: handle,
                    },
                )
                .context("ターゲットを登録できない")?;

            println!("登録: {} ({})", target.display_name, target.slug);
            println!("  ハンドル: {}", target.handles.join(", "));
            println!(
                "  last_seen_rowid = {} （バックログ保護）",
                target.last_seen_rowid.unwrap_or(0)
            );
            println!("  自動送信: OFF");
            println!();
            println!("この時点より前のメッセージは処理対象になりません。");
        }

        TargetCmd::List => {
            let targets = store.list_targets()?;
            if targets.is_empty() {
                println!("登録なし。`target add` で追加する。");
                return Ok(());
            }
            for t in targets {
                println!(
                    "{:<12} {:<10} 自動送信:{:<4} last_seen:{:<8} {}",
                    t.slug,
                    t.display_name,
                    if t.auto_send { "ON" } else { "OFF" },
                    t.last_seen_rowid.unwrap_or(0),
                    if t.enabled { "" } else { "(無効)" }
                );
                println!("             {}", t.handles.join(", "));
            }
        }

        TargetCmd::Rm { slug } => {
            let target = store
                .target_by_slug(&slug)?
                .with_context(|| format!("'{slug}' は登録されていない"))?;
            store.remove_target(target.id)?;
            println!("削除: {} ({})", target.display_name, target.slug);
        }

        TargetCmd::Set { slug, preset } => {
            let target = store
                .target_by_slug(&slug)?
                .with_context(|| format!("'{slug}' は登録されていない"))?;
            if let Some(p) = &preset {
                LengthPreset::parse(p).with_context(|| format!("不明な長さ指定: {p}"))?;
                store.set_reply_preset(target.id, p)?;
            }
            let t = store.target_by_slug(&slug)?.unwrap();
            println!("{}", t.display_name);
            println!("  長さ: {}", t.reply_preset);
        }

        TargetCmd::Pending { slug } => {
            let target = store
                .target_by_slug(&slug)?
                .with_context(|| format!("'{slug}' は登録されていない"))?;
            let after = target.last_seen_rowid.unwrap_or(0);
            let new = imessage::messages_after(chat_db, &target.handles, after)?;

            println!(
                "{} ({}) last_seen_rowid = {}",
                target.display_name, target.slug, after
            );
            let actionable: Vec<_> = new.iter().filter(|m| m.skip.is_none() && !m.is_from_me).collect();
            println!("新着 {} 件 / うち生成対象 {} 件", new.len(), actionable.len());
            for m in actionable {
                println!(
                    "  #{} {} {}",
                    m.rowid,
                    m.date.format("%m-%d %H:%M"),
                    m.body.as_deref().unwrap_or("")
                );
            }
        }
    }
    Ok(())
}
