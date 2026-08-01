//! chat.db の行をメッセージに変換する。
//!
//! `message.text` は macOS Ventura 以降ほぼ常に NULL である（仕様書 5.1 / 14.1）。
//! 実測では対象の会話 5839 件すべてが NULL だった。本文は `attributedBody`
//! （typedstream）からデコードするのが唯一の実用経路で、`text` は保険にすぎない。

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use imessage_database::util::{
    dates::{get_local_time, get_offset},
    streamtyped,
};
use rusqlite::{Connection, Row};

/// 添付ファイル 1 件につき 1 つ入る。
const OBJECT_REPLACEMENT: char = '\u{FFFC}';
/// App メッセージで入る。
const REPLACEMENT: char = '\u{FFFD}';

/// 生成対象から除外する理由（仕様書 5.1「除外すべき行」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// 相手が送信取り消しした（`date_retracted`）。
    Retracted,
    /// タップバック・リアクション（`associated_message_type != 0`）。
    Tapback,
    /// 参加者追加などのシステムメッセージ（`item_type != 0`）。
    SystemItem,
    /// ステッカー・App メッセージ（`balloon_bundle_id IS NOT NULL`）。
    AppMessage,
    /// 本文が空（添付のみ）。
    EmptyBody,
}

impl SkipReason {
    pub fn label(self) -> &'static str {
        match self {
            SkipReason::Retracted => "retracted",
            SkipReason::Tapback => "tapback",
            SkipReason::SystemItem => "system",
            SkipReason::AppMessage => "app_message",
            SkipReason::EmptyBody => "empty_body",
        }
    }
}

/// chat.db から読んだ 1 メッセージ。
#[derive(Debug, Clone)]
pub struct Message {
    pub rowid: i64,
    pub guid: String,
    /// 受信元の `chat_identifier`。**返信は必ずこの値に送る**（仕様書 6.3）。
    pub chat_identifier: String,
    pub date: DateTime<Local>,
    pub is_from_me: bool,
    pub edited: bool,
    /// デコード済み本文。除外対象でも取れていれば入る。
    pub body: Option<String>,
    /// `None` なら生成対象。
    pub skip: Option<SkipReason>,
    /// 本文が `attributedBody` ではなく `text` から取れたか（デバッグ用）。
    pub body_from_text_column: bool,
}

/// 会話相手の一覧（本文は一切読まない。メタデータのみ）。
#[derive(Debug, Clone)]
pub struct ChatSummary {
    pub chat_identifier: String,
    pub service_name: String,
    pub display_name: String,
    pub message_count: i64,
    pub last_message: Option<DateTime<Local>>,
}

/// 仕様書 5.1 のクエリ。`handle` テーブル経由ではなく
/// `chat_message_join` → `chat` 経由でないと `handle_id = 0` になる自分の送信を
/// 取りこぼす（仕様書 14.2）。実測で自分の送信 1186 件のうち 782 件が該当した。
const SELECT_COLUMNS: &str = "
    m.ROWID,
    m.guid,
    m.text,
    m.attributedBody,
    m.date,
    m.is_from_me,
    m.item_type,
    m.associated_message_type,
    m.balloon_bundle_id,
    m.date_retracted,
    m.date_edited,
    c.chat_identifier
FROM message m
JOIN chat_message_join cmj ON cmj.message_id = m.ROWID
JOIN chat c               ON c.ROWID = cmj.chat_id
";

fn placeholders(n: usize) -> String {
    std::iter::repeat("?").take(n).collect::<Vec<_>>().join(", ")
}

/// 指定ハンドルの直近 `limit` 件を、古い順に返す。
///
/// allowlist は SQL の WHERE 句で効かせる。取得してからフィルタするのではなく
/// そもそも読み込まない（仕様書 6.4.1）。
pub fn recent_messages(conn: &Connection, handles: &[String], limit: u32) -> Result<Vec<Message>> {
    if handles.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT {SELECT_COLUMNS} WHERE c.chat_identifier IN ({}) \
         ORDER BY m.ROWID DESC LIMIT ?",
        placeholders(handles.len())
    );

    let mut params: Vec<&dyn rusqlite::ToSql> =
        handles.iter().map(|h| h as &dyn rusqlite::ToSql).collect();
    params.push(&limit);

    let mut out = collect(conn, &sql, params.as_slice())?;
    // DESC で取ったので古い順に戻す。
    out.reverse();
    Ok(out)
}

/// `after_rowid` より新しいメッセージを古い順に返す。watcher の主経路。
pub fn messages_after(
    conn: &Connection,
    handles: &[String],
    after_rowid: i64,
) -> Result<Vec<Message>> {
    if handles.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT {SELECT_COLUMNS} WHERE c.chat_identifier IN ({}) AND m.ROWID > ? \
         ORDER BY m.ROWID ASC",
        placeholders(handles.len())
    );

    let mut params: Vec<&dyn rusqlite::ToSql> =
        handles.iter().map(|h| h as &dyn rusqlite::ToSql).collect();
    params.push(&after_rowid);

    collect(conn, &sql, params.as_slice())
}

/// 指定ハンドル群の現在の最大 ROWID。
///
/// **バックログ保護（仕様書 6.1）の要**。ターゲット登録時にこの値を
/// `last_seen_rowid` に入れることで、過去のメッセージを一切処理させない。
pub fn max_rowid(conn: &Connection, handles: &[String]) -> Result<Option<i64>> {
    if handles.is_empty() {
        return Ok(None);
    }
    let sql = format!(
        "SELECT MAX(m.ROWID)
         FROM message m
         JOIN chat_message_join cmj ON cmj.message_id = m.ROWID
         JOIN chat c               ON c.ROWID = cmj.chat_id
         WHERE c.chat_identifier IN ({})",
        placeholders(handles.len())
    );
    let params: Vec<&dyn rusqlite::ToSql> =
        handles.iter().map(|h| h as &dyn rusqlite::ToSql).collect();

    let max: Option<i64> = conn.query_row(&sql, params.as_slice(), |row| row.get(0))?;
    Ok(max)
}

/// 自分が `after_rowid` より後に送信した件数（仕様書 6.4.3 既返信チェック）。
///
/// 1 以上ならスキップする。生成の直前と送信の直前の 2 回呼ぶこと。
pub fn count_own_replies_after(
    conn: &Connection,
    handles: &[String],
    after_rowid: i64,
) -> Result<i64> {
    if handles.is_empty() {
        return Ok(0);
    }
    let sql = format!(
        "SELECT COUNT(*)
         FROM message m
         JOIN chat_message_join cmj ON cmj.message_id = m.ROWID
         JOIN chat c               ON c.ROWID = cmj.chat_id
         WHERE c.chat_identifier IN ({})
           AND m.is_from_me = 1
           AND m.ROWID > ?",
        placeholders(handles.len())
    );
    let mut params: Vec<&dyn rusqlite::ToSql> =
        handles.iter().map(|h| h as &dyn rusqlite::ToSql).collect();
    params.push(&after_rowid);

    let count: i64 = conn.query_row(&sql, params.as_slice(), |row| row.get(0))?;
    Ok(count)
}

/// 会話相手の一覧を返す。ターゲットを選ばせるための補助（仕様書 10.2-3）。
/// 本文（`text` / `attributedBody`）は SELECT しない。
pub fn list_chats(conn: &Connection, limit: u32) -> Result<Vec<ChatSummary>> {
    let mut stmt = conn.prepare(
        "SELECT c.chat_identifier,
                COALESCE(c.service_name, ''),
                COALESCE(c.display_name, ''),
                COUNT(m.ROWID),
                MAX(m.date)
         FROM chat c
         JOIN chat_message_join cmj ON cmj.chat_id = c.ROWID
         JOIN message m            ON m.ROWID = cmj.message_id
         GROUP BY c.chat_identifier, c.service_name
         ORDER BY MAX(m.date) DESC
         LIMIT ?",
    )?;

    let offset = get_offset();
    let rows = stmt.query_map([limit], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Option<i64>>(4)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (chat_identifier, service_name, display_name, message_count, last_date) = row?;
        out.push(ChatSummary {
            chat_identifier,
            service_name,
            display_name,
            message_count,
            last_message: last_date.and_then(|d| get_local_time(d, offset).ok()),
        });
    }
    Ok(out)
}

// MARK: 内部

/// 行の生の値。日時変換が失敗しうるので一度こちらで受ける。
type RawRow = (
    i64,
    String,
    Option<String>,
    Option<Vec<u8>>,
    i64,
    i64,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    String,
);

fn read_raw(row: &Row) -> rusqlite::Result<RawRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
    ))
}

fn collect(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Result<Vec<Message>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, read_raw)?;

    let offset = get_offset();
    let mut out = Vec::new();
    for row in rows {
        let (
            rowid,
            guid,
            text,
            attributed_body,
            date,
            is_from_me,
            item_type,
            associated_message_type,
            balloon_bundle_id,
            date_retracted,
            date_edited,
            chat_identifier,
        ) = row?;

        let (body, body_from_text_column) =
            decode_body(attributed_body.as_deref(), text.as_deref());

        let skip = classify(
            date_retracted.unwrap_or(0),
            associated_message_type.unwrap_or(0),
            item_type.unwrap_or(0),
            balloon_bundle_id.as_deref(),
            body.as_deref(),
        );

        out.push(Message {
            rowid,
            guid,
            chat_identifier,
            date: get_local_time(date, offset)
                .with_context(|| format!("ROWID {rowid} の日時を変換できない: {date}"))?,
            is_from_me: is_from_me != 0,
            edited: date_edited.unwrap_or(0) != 0,
            body,
            skip,
            body_from_text_column,
        });
    }
    Ok(out)
}

/// 本文を取り出す。返り値の `bool` は `text` カラムから取れたかどうか。
///
/// 優先順（仕様書 5.1「本文の取り出し」）:
/// 1. `attributedBody` を typedstream としてデコード
/// 2. 失敗したら `m.text`
/// 3. 両方空なら `None`
fn decode_body(attributed_body: Option<&[u8]>, text: Option<&str>) -> (Option<String>, bool) {
    if let Some(bytes) = attributed_body {
        if let Ok(decoded) = streamtyped::parse(bytes.to_vec()) {
            let cleaned = clean(&decoded);
            if !cleaned.is_empty() {
                return (Some(cleaned), false);
            }
        }
    }

    if let Some(raw) = text {
        let cleaned = clean(raw);
        if !cleaned.is_empty() {
            return (Some(cleaned), true);
        }
    }

    (None, false)
}

/// デコード結果に混ざる制御文字を除去する。
fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| *c != OBJECT_REPLACEMENT && *c != REPLACEMENT)
        .collect::<String>()
        .trim()
        .to_string()
}

fn classify(
    date_retracted: i64,
    associated_message_type: i64,
    item_type: i64,
    balloon_bundle_id: Option<&str>,
    body: Option<&str>,
) -> Option<SkipReason> {
    if date_retracted != 0 {
        return Some(SkipReason::Retracted);
    }
    if associated_message_type != 0 {
        return Some(SkipReason::Tapback);
    }
    if item_type != 0 {
        return Some(SkipReason::SystemItem);
    }
    if balloon_bundle_id.is_some() {
        return Some(SkipReason::AppMessage);
    }
    if body.is_none() {
        return Some(SkipReason::EmptyBody);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_strips_attachment_placeholders() {
        assert_eq!(clean("\u{FFFC}写真だよ\u{FFFC}"), "写真だよ");
        assert_eq!(clean("  はい  "), "はい");
        assert_eq!(clean("\u{FFFC}"), "");
    }

    #[test]
    fn text_column_is_only_a_fallback() {
        // attributedBody がデコードできない場合だけ text を使う。
        let (body, from_text) = decode_body(Some(&[0, 1, 2]), Some("フォールバック"));
        assert_eq!(body.as_deref(), Some("フォールバック"));
        assert!(from_text);

        // 添付のみのメッセージは本文なしとして扱う。
        let (body, _) = decode_body(None, Some("\u{FFFC}"));
        assert_eq!(body, None);
    }

    #[test]
    fn classify_matches_spec_table() {
        assert_eq!(classify(0, 0, 0, None, Some("やあ")), None);
        assert_eq!(classify(1, 0, 0, None, Some("やあ")), Some(SkipReason::Retracted));
        assert_eq!(classify(0, 2000, 0, None, Some("やあ")), Some(SkipReason::Tapback));
        assert_eq!(classify(0, 0, 1, None, Some("やあ")), Some(SkipReason::SystemItem));
        assert_eq!(
            classify(0, 0, 0, Some("com.apple.messages.URLBalloonProvider"), Some("やあ")),
            Some(SkipReason::AppMessage)
        );
        assert_eq!(classify(0, 0, 0, None, None), Some(SkipReason::EmptyBody));
    }

    #[test]
    fn placeholders_builds_bind_list() {
        assert_eq!(placeholders(3), "?, ?, ?");
        assert_eq!(placeholders(1), "?");
    }
}
