//! 検証・運用 CLI。Phase 2 で Tauri UI を作るまでの操作口を兼ねる。

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use momreply_core::{
    fewshot, imessage,
    pipeline::{self, LengthPreset},
    profile,
    questions::{self, QuestionKind},
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
    /// 相手からの質問と、それに対する自分の答えを管理する。
    #[command(subcommand)]
    Questions(QuestionCmd),
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
        /// mirror|short|normal|long|very_long
        #[arg(long, default_value = "mirror")]
        length: String,
    },
}

#[derive(Subcommand)]
enum QuestionCmd {
    /// 直近のメッセージから質問を抽出して溜める。
    Scan {
        #[arg(long)]
        slug: String,
        /// 遡る件数。
        #[arg(long, default_value_t = 100)]
        limit: u32,
    },
    /// まだ答えていない質問を並べる。
    List {
        #[arg(long)]
        slug: String,
    },
    /// 質問に答える。答えは self.md にも追記され、次からは聞かれない。
    Answer {
        #[arg(long)]
        id: i64,
        #[arg(long)]
        answer: String,
    },
    /// 「明日来る？」のような、その都度聞かれるが答えが一貫している
    /// 質問に、既定の答えを設定する。
    Standing {
        #[arg(long)]
        slug: String,
        /// 既定の答え。省略すると現在の設定を表示する。
        #[arg(long)]
        set: Option<String>,
        /// この定型回答での自動送信を承認する（初回のみ必要）。
        #[arg(long)]
        confirm: bool,
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
        Command::Questions(cmd) => cmd_questions(&chat_db, cmd),
        Command::Config { primary, user_name } => cmd_config(primary, user_name),
        Command::Fewshot { slug, limit, scan } => cmd_fewshot(&chat_db, &slug, limit, scan),
        Command::Generate {
            slug,
            rowid,
            length,
        } => cmd_generate(&chat_db, &slug, rowid, &length).await,
    }
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
    length: &str,
) -> Result<()> {
    let store = Store::open_default()?;
    let target = store
        .target_by_slug(slug)?
        .with_context(|| format!("'{slug}' は登録されていない"))?;
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
    for line in message.body.as_deref().unwrap_or("").lines() {
        println!("  {line}");
    }
    println!();

    let draft = pipeline::draft_reply(chat_db, &store, &target, &message, preset).await?;

    if !draft.unanswerable.is_empty() {
        println!("答える材料がありません。生成せずに確認へ回しました。");
        for q in &draft.unanswerable {
            println!("  ・{q}");
        }
        println!();
        println!("答えを登録する:");
        println!("  momreply-cli questions list --slug {slug}");
        store.record_processed(
            target.id,
            message.rowid,
            &message.chat_identifier,
            message.date.timestamp(),
            message.body.as_deref(),
            "awaiting_review",
            Some("needs_answer"),
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

    store.record_processed(
        target.id,
        message.rowid,
        &message.chat_identifier,
        message.date.timestamp(),
        message.body.as_deref(),
        if draft.held_for_review { "awaiting_review" } else { "dry_run" },
        None,
        Some(&draft.text),
        Some(&draft.provider),
        Some(&draft.model),
    )?;
    Ok(())
}

fn cmd_questions(chat_db: &rusqlite::Connection, cmd: QuestionCmd) -> Result<()> {
    let store = Store::open_default()?;

    match cmd {
        QuestionCmd::Scan { slug, limit } => {
            let target = store
                .target_by_slug(&slug)?
                .with_context(|| format!("'{slug}' は登録されていない"))?;

            let messages = imessage::recent_messages(chat_db, &target.handles, limit)?;
            let mut scanned = 0usize;
            let mut added = 0usize;
            let mut already_known = 0usize;
            let mut visit = 0usize;

            for m in &messages {
                // 自分の発言と除外対象は見ない。
                if m.is_from_me || m.skip.is_some() {
                    continue;
                }
                let Some(body) = &m.body else { continue };

                let found = questions::extract(body);
                if found.is_empty() {
                    continue;
                }
                scanned += found.len();

                for q in &found {
                    if q.kind() == QuestionKind::Visit {
                        visit += 1;
                    }
                    if store.known_answer(target.id, &q.text)?.is_some() {
                        already_known += 1;
                    }
                }
                added += store.record_questions(target.id, m.rowid, &found)?;
            }

            println!(
                "{} 件のメッセージから質問 {} 件を検出",
                messages.len(),
                scanned
            );
            println!("  新規に記録: {added} 件");
            println!("  既に答えがある: {already_known} 件");
            println!("  予定型（定型回答で扱う）: {visit} 件");

            if visit > 0 && store.standing_answer(target.id, QuestionKind::Visit)?.is_none() {
                println!();
                println!(
                    "予定型の質問に既定の答えが未設定です。設定するまで毎回確認が必要になります:"
                );
                println!(
                    "  momreply-cli questions standing --slug {slug} --set \"行かない\""
                );
            }
            if added > 0 {
                println!();
                println!("`momreply-cli questions list --slug {slug}` で確認して答える。");
            }
        }

        QuestionCmd::List { slug } => {
            let target = store
                .target_by_slug(&slug)?
                .with_context(|| format!("'{slug}' は登録されていない"))?;
            let pending = store.unanswered_questions(target.id)?;

            if pending.is_empty() {
                println!("未回答の質問はありません。");
                return Ok(());
            }
            println!("未回答 {} 件:", pending.len());
            for q in &pending {
                println!("  #{:<4} {}", q.id, q.question);
                if let Some(ctx) = &q.context {
                    let brief: String = ctx.chars().take(60).collect();
                    let ellipsis = if ctx.chars().count() > 60 { "…" } else { "" };
                    println!("        （状況: {brief}{ellipsis}）");
                }
            }
            println!();
            println!("答える: momreply-cli questions answer --id <ID> --answer \"...\"");
        }

        QuestionCmd::Answer { id, answer } => {
            let q = store.answer_question(id, &answer)?;
            profile::append_fact(&q.question, &answer)?;
            println!("記録しました。");
            println!("  質問: {}", q.question);
            println!("  答え: {answer}");
            println!();
            println!("self.md: {}", momreply_core::paths::self_profile()?.display());
            println!("次から同じ質問が来ても、あなたに聞かずに答えられます。");
        }

        QuestionCmd::Standing { slug, set, confirm } => {
            let target = store
                .target_by_slug(&slug)?
                .with_context(|| format!("'{slug}' は登録されていない"))?;

            if let Some(answer) = set {
                let saved = store.set_standing_answer(target.id, QuestionKind::Visit, &answer)?;
                println!("予定型の質問への既定の答えを設定しました。");
                println!("  「明日来る？」「泊まる？」などに対して: 「{}」", saved.answer);
                println!();
                if saved.is_confirmed() {
                    println!("承認済みのため、自動送信に使われます。");
                } else {
                    println!("まだ自動送信には使いません。内容を確認したうえで承認してください:");
                    println!("  momreply-cli questions standing --slug {slug} --confirm");
                }
                return Ok(());
            }

            if confirm {
                store.confirm_standing_answer(target.id, QuestionKind::Visit)?;
                println!("承認しました。以後この定型回答は自動送信に使われます。");
                println!("変更したいときは --set で上書きすると、承認は取り消されます。");
                return Ok(());
            }

            let answers = store.list_standing_answers(target.id)?;
            if answers.is_empty() {
                println!("定型回答は未設定です。");
                println!("  momreply-cli questions standing --slug {slug} --set \"行かない\"");
                return Ok(());
            }
            for a in answers {
                println!(
                    "{:<8} 「{}」  {}",
                    format!("{:?}", a.kind),
                    a.answer,
                    if a.is_confirmed() { "承認済み" } else { "未承認（自動送信しない）" }
                );
            }
        }
    }
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
