//! 検証・運用 CLI。Phase 2 で Tauri UI を作るまでの操作口を兼ねる。

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use momreply_core::{
    imessage,
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

fn main() -> Result<()> {
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
    }
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
