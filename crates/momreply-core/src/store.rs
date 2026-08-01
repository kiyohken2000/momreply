//! app.db のスキーマと CRUD（仕様書 5.2 を targets 軸に正規化したもの）。
//!
//! 仕様書は監視対象を「母」1 人に固定し `target.handles` を config に置いていたが、
//! 対象を任意に選べるようにするため、相手を第一級のエンティティにしている。
//! プロファイル・few-shot・連続カウンタ・レート制限はすべて相手ごとに要るため、
//! 配列 1 本では足りない。

use std::path::Path;

use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OptionalExtension};

use crate::imessage;

/// スキーマバージョン。`PRAGMA user_version` で管理する。
const SCHEMA_VERSION: i32 = 1;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS targets (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  slug          TEXT    NOT NULL UNIQUE,          -- プロファイルのファイル名に使う
  display_name  TEXT    NOT NULL,
  enabled       INTEGER NOT NULL DEFAULT 1,
  -- 既定は OFF。配布時に自動送信が既定で走る状態にしてはいけない。
  auto_send     INTEGER NOT NULL DEFAULT 0,
  reply_preset  TEXT    NOT NULL DEFAULT 'mirror',
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS target_handles (
  target_id       INTEGER NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
  chat_identifier TEXT    NOT NULL,
  PRIMARY KEY (target_id, chat_identifier)
);
-- 1 つのハンドルが 2 人のターゲットに属すると、どちらの人格で返すか決まらない。
CREATE UNIQUE INDEX IF NOT EXISTS idx_target_handles_unique
  ON target_handles(chat_identifier);

CREATE TABLE IF NOT EXISTS target_state (
  target_id              INTEGER PRIMARY KEY REFERENCES targets(id) ON DELETE CASCADE,
  -- バックログ保護の要（仕様書 6.1）。ターゲット追加時に必ず埋まる。
  last_seen_rowid        INTEGER,
  consecutive_auto_count INTEGER NOT NULL DEFAULT 0,
  session_started_at     INTEGER,
  last_sent_at           INTEGER,
  length_preset_override TEXT
);

CREATE TABLE IF NOT EXISTS processed_messages (
  chat_rowid   INTEGER PRIMARY KEY,               -- chat.db の message.ROWID
  target_id    INTEGER NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
  chat_guid    TEXT    NOT NULL,                  -- 受信元 chat_identifier（仕様書 6.3）
  received_at  INTEGER NOT NULL,
  body         TEXT,
  status       TEXT    NOT NULL,                  -- pending|generating|awaiting_review|sent|skipped|failed|dry_run
  skip_reason  TEXT,
  draft        TEXT,
  final_text   TEXT,
  sent_at      INTEGER,
  sent_rowid   INTEGER,
  provider     TEXT,
  model        TEXT,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_processed_target
  ON processed_messages(target_id, status);

CREATE TABLE IF NOT EXISTS generation_log (
  id               INTEGER PRIMARY KEY AUTOINCREMENT,
  target_id        INTEGER REFERENCES targets(id) ON DELETE SET NULL,
  chat_rowid       INTEGER,
  kind             TEXT NOT NULL,                 -- initial|regenerate|profile_update|classification
  provider         TEXT,
  model            TEXT,
  input_tokens     INTEGER,
  output_tokens    INTEGER,
  latency_ms       INTEGER,
  user_instruction TEXT,
  output           TEXT,
  error            TEXT,
  created_at       INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS profile_revisions (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  target_id     INTEGER NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
  content       TEXT    NOT NULL,
  source        TEXT    NOT NULL,                 -- manual|ai
  change_note   TEXT,
  source_rowids TEXT,
  applied       INTEGER NOT NULL DEFAULT 0,
  created_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS fewshot_pairs (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  target_id    INTEGER NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
  incoming     TEXT    NOT NULL,
  reply        TEXT    NOT NULL,
  source_rowid INTEGER NOT NULL,
  built_at     INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_fewshot_target ON fewshot_pairs(target_id);

-- 仕様書には無い。相手の質問に具体的に答えるために必要になる。
-- 答える材料が self.md に無い質問をここに溜め、一度だけ人間に聞く。
-- 答えは self.md に反映され、以後は同じ質問を自動で答えられるようになる。
CREATE TABLE IF NOT EXISTS pending_questions (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  target_id   INTEGER NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
  chat_rowid  INTEGER,
  question    TEXT    NOT NULL,
  answer      TEXT,
  answered_at INTEGER,
  created_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_pending_unanswered
  ON pending_questions(target_id, answered_at);

CREATE TABLE IF NOT EXISTS kv (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
"#;

/// 登録済みの相手。
#[derive(Debug, Clone)]
pub struct Target {
    pub id: i64,
    pub slug: String,
    pub display_name: String,
    pub enabled: bool,
    pub auto_send: bool,
    pub reply_preset: String,
    pub handles: Vec<String>,
    pub last_seen_rowid: Option<i64>,
}

/// 新規登録の入力。
#[derive(Debug, Clone)]
pub struct NewTarget {
    pub slug: String,
    pub display_name: String,
    /// 電話番号と Apple ID で会話が 2 本に分かれている場合は両方入れる。
    pub handles: Vec<String>,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// 既定パス（`~/Library/Application Support/net.votepurchase.momreply/app.db`）で開く。
    pub fn open_default() -> Result<Self> {
        crate::paths::ensure_dirs()?;
        Self::open(&crate::paths::app_db()?)
    }

    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("app.db を開けない: {}", path.display()))?;
        Self::init(conn)
    }

    /// テスト用。
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        let mut store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<()> {
        let version: i32 =
            self.conn
                .pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            bail!(
                "app.db のスキーマバージョン {version} は、このビルドが知っている {SCHEMA_VERSION} より新しい。\
                 古いバージョンのアプリで開いている可能性がある"
            );
        }
        self.conn.execute_batch(SCHEMA)?;
        self.conn
            .pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(())
    }

    /// 相手を登録する。
    ///
    /// # バックログ保護（仕様書 6.1）
    ///
    /// 登録と同時に、その時点の chat.db 上の最大 ROWID を `last_seen_rowid` に
    /// 書き込む。これにより**過去のメッセージは一切処理対象にならない**。
    ///
    /// 仕様書はこれを「初回起動時」の話として書いているが、対象を任意に選べる
    /// 設計では**相手を追加するたびに同じ事故が起きる**（母を登録した瞬間に
    /// 5839 件が新着扱いになる）。そのため保護をこの関数の内側に閉じ込め、
    /// 呼び出し側がスキップできる経路を作っていない。
    /// 引数の `chat_db` はそのために必須である。
    pub fn add_target(&mut self, chat_db: &Connection, new: NewTarget) -> Result<Target> {
        if new.handles.is_empty() {
            bail!("ハンドルを 1 つ以上指定すること");
        }
        if new.slug.trim().is_empty() {
            bail!("slug が空");
        }

        // 先に chat.db を読む。トランザクションの外で確定させておく。
        let backlog_guard_rowid = imessage::max_rowid(chat_db, &new.handles)?.unwrap_or(0);

        let now = now_unix();
        let tx = self.conn.transaction()?;

        tx.execute(
            "INSERT INTO targets (slug, display_name, enabled, auto_send, reply_preset, created_at, updated_at)
             VALUES (?1, ?2, 1, 0, 'mirror', ?3, ?3)",
            (&new.slug, &new.display_name, now),
        )
        .with_context(|| format!("ターゲット '{}' を作れない（slug が重複している可能性）", new.slug))?;
        let target_id = tx.last_insert_rowid();

        for handle in &new.handles {
            tx.execute(
                "INSERT INTO target_handles (target_id, chat_identifier) VALUES (?1, ?2)",
                (target_id, handle),
            )
            .with_context(|| {
                format!("ハンドル '{handle}' を登録できない（他のターゲットが既に使っている可能性）")
            })?;
        }

        tx.execute(
            "INSERT INTO target_state (target_id, last_seen_rowid) VALUES (?1, ?2)",
            (target_id, backlog_guard_rowid),
        )?;

        tx.commit()?;

        self.target_by_id(target_id)?
            .context("登録直後のターゲットを読み出せない")
    }

    pub fn target_by_id(&self, id: i64) -> Result<Option<Target>> {
        let row = self
            .conn
            .query_row(
                "SELECT t.id, t.slug, t.display_name, t.enabled, t.auto_send, t.reply_preset,
                        s.last_seen_rowid
                 FROM targets t
                 LEFT JOIN target_state s ON s.target_id = t.id
                 WHERE t.id = ?1",
                [id],
                Self::read_target_row,
            )
            .optional()?;

        match row {
            Some(mut t) => {
                t.handles = self.handles_of(t.id)?;
                Ok(Some(t))
            }
            None => Ok(None),
        }
    }

    pub fn target_by_slug(&self, slug: &str) -> Result<Option<Target>> {
        let id: Option<i64> = self
            .conn
            .query_row("SELECT id FROM targets WHERE slug = ?1", [slug], |r| r.get(0))
            .optional()?;
        match id {
            Some(id) => self.target_by_id(id),
            None => Ok(None),
        }
    }

    pub fn list_targets(&self) -> Result<Vec<Target>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.slug, t.display_name, t.enabled, t.auto_send, t.reply_preset,
                    s.last_seen_rowid
             FROM targets t
             LEFT JOIN target_state s ON s.target_id = t.id
             ORDER BY t.id",
        )?;
        let rows = stmt.query_map([], Self::read_target_row)?;

        let mut out = Vec::new();
        for row in rows {
            let mut t = row?;
            t.handles = self.handles_of(t.id)?;
            out.push(t);
        }
        Ok(out)
    }

    /// 有効なターゲット全員のハンドルの和集合。watcher の allowlist に使う。
    ///
    /// この結果を SQL の WHERE 句に流し込むことで、対象外の相手のメッセージは
    /// そもそも読み込まれない（仕様書 6.4.1）。
    pub fn enabled_handles(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT h.chat_identifier
             FROM target_handles h
             JOIN targets t ON t.id = h.target_id
             WHERE t.enabled = 1
             ORDER BY h.chat_identifier",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// `chat_identifier` からターゲットを引く。受信メッセージの振り分けに使う。
    pub fn target_for_handle(&self, chat_identifier: &str) -> Result<Option<Target>> {
        let id: Option<i64> = self
            .conn
            .query_row(
                "SELECT target_id FROM target_handles WHERE chat_identifier = ?1",
                [chat_identifier],
                |r| r.get(0),
            )
            .optional()?;
        match id {
            Some(id) => self.target_by_id(id),
            None => Ok(None),
        }
    }

    pub fn set_last_seen_rowid(&self, target_id: i64, rowid: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE target_state SET last_seen_rowid = ?2 WHERE target_id = ?1",
            (target_id, rowid),
        )?;
        Ok(())
    }

    pub fn set_auto_send(&self, target_id: i64, enabled: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE targets SET auto_send = ?2, updated_at = ?3 WHERE id = ?1",
            (target_id, enabled, now_unix()),
        )?;
        Ok(())
    }

    pub fn remove_target(&self, target_id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM targets WHERE id = ?1", [target_id])?;
        Ok(())
    }

    fn handles_of(&self, target_id: i64) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT chat_identifier FROM target_handles WHERE target_id = ?1 ORDER BY chat_identifier",
        )?;
        let rows = stmt.query_map([target_id], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn read_target_row(row: &rusqlite::Row) -> rusqlite::Result<Target> {
        Ok(Target {
            id: row.get(0)?,
            slug: row.get(1)?,
            display_name: row.get(2)?,
            enabled: row.get::<_, i64>(3)? != 0,
            auto_send: row.get::<_, i64>(4)? != 0,
            reply_preset: row.get(5)?,
            last_seen_rowid: row.get(6)?,
            handles: Vec::new(),
        })
    }
}

fn now_unix() -> i64 {
    chrono::Local::now().timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// chat.db の最小構造を再現したダミーを作る。
    fn fake_chat_db(handle: &str, message_count: i64) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE message (
                ROWID INTEGER PRIMARY KEY, guid TEXT, text TEXT, attributedBody BLOB,
                date INTEGER, is_from_me INTEGER, item_type INTEGER,
                associated_message_type INTEGER, balloon_bundle_id TEXT,
                date_retracted INTEGER, date_edited INTEGER);
             CREATE TABLE chat (ROWID INTEGER PRIMARY KEY, chat_identifier TEXT,
                service_name TEXT, display_name TEXT);
             CREATE TABLE chat_message_join (chat_id INTEGER, message_id INTEGER);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chat (ROWID, chat_identifier, service_name, display_name)
             VALUES (1, ?1, 'iMessage', '')",
            [handle],
        )
        .unwrap();
        for i in 1..=message_count {
            conn.execute(
                "INSERT INTO message (ROWID, guid, text, date, is_from_me, item_type,
                    associated_message_type, date_retracted, date_edited)
                 VALUES (?1, ?2, '過去のメッセージ', 700000000000000000, 0, 0, 0, 0, 0)",
                (i, format!("guid-{i}")),
            )
            .unwrap();
            conn.execute(
                "INSERT INTO chat_message_join (chat_id, message_id) VALUES (1, ?1)",
                [i],
            )
            .unwrap();
        }
        conn
    }

    fn new_target(slug: &str, handle: &str) -> NewTarget {
        NewTarget {
            slug: slug.to_string(),
            display_name: slug.to_string(),
            handles: vec![handle.to_string()],
        }
    }

    /// Phase 1 の受け入れ基準:
    /// 「初回起動時に過去メッセージが一切処理されない」。
    #[test]
    fn adding_a_target_never_exposes_history() {
        let chat_db = fake_chat_db("mom@example.com", 5839);
        let mut store = Store::open_in_memory().unwrap();

        let target = store
            .add_target(&chat_db, new_target("mom", "mom@example.com"))
            .unwrap();

        // 登録直後の last_seen_rowid が、履歴の最大 ROWID に一致している。
        assert_eq!(target.last_seen_rowid, Some(5839));

        // したがって「新着」は 0 件。5839 件が一斉処理されることはない。
        let new_messages = imessage::messages_after(
            &chat_db,
            &target.handles,
            target.last_seen_rowid.unwrap(),
        )
        .unwrap();
        assert!(
            new_messages.is_empty(),
            "登録直後に {} 件が新着扱いになっている",
            new_messages.len()
        );
    }

    /// 会話が 1 件も無い相手を登録しても、あとから届く分は拾える。
    #[test]
    fn empty_history_starts_from_zero() {
        let chat_db = fake_chat_db("new@example.com", 0);
        let mut store = Store::open_in_memory().unwrap();
        let target = store
            .add_target(&chat_db, new_target("new", "new@example.com"))
            .unwrap();
        assert_eq!(target.last_seen_rowid, Some(0));
    }

    #[test]
    fn a_handle_cannot_belong_to_two_targets() {
        let chat_db = fake_chat_db("shared@example.com", 3);
        let mut store = Store::open_in_memory().unwrap();
        store
            .add_target(&chat_db, new_target("first", "shared@example.com"))
            .unwrap();
        let err = store
            .add_target(&chat_db, new_target("second", "shared@example.com"))
            .unwrap_err();
        assert!(err.to_string().contains("shared@example.com"), "{err}");
    }

    #[test]
    fn allowlist_covers_only_enabled_targets() {
        let chat_db = fake_chat_db("a@example.com", 2);
        let mut store = Store::open_in_memory().unwrap();
        let a = store
            .add_target(&chat_db, new_target("a", "a@example.com"))
            .unwrap();
        store
            .add_target(
                &chat_db,
                NewTarget {
                    slug: "b".into(),
                    display_name: "b".into(),
                    handles: vec!["b@example.com".into(), "+815000000000".into()],
                },
            )
            .unwrap();

        assert_eq!(store.enabled_handles().unwrap().len(), 3);

        store
            .conn
            .execute("UPDATE targets SET enabled = 0 WHERE id = ?1", [a.id])
            .unwrap();
        let handles = store.enabled_handles().unwrap();
        assert_eq!(handles.len(), 2);
        assert!(!handles.contains(&"a@example.com".to_string()));
    }

    #[test]
    fn auto_send_defaults_to_off() {
        let chat_db = fake_chat_db("x@example.com", 1);
        let mut store = Store::open_in_memory().unwrap();
        let t = store
            .add_target(&chat_db, new_target("x", "x@example.com"))
            .unwrap();
        assert!(!t.auto_send, "配布物で自動送信が既定 ON になってはいけない");
    }

    #[test]
    fn handle_maps_back_to_its_target() {
        let chat_db = fake_chat_db("m@example.com", 5);
        let mut store = Store::open_in_memory().unwrap();
        store
            .add_target(
                &chat_db,
                NewTarget {
                    slug: "mom".into(),
                    display_name: "母".into(),
                    handles: vec!["m@example.com".into(), "+819000000000".into()],
                },
            )
            .unwrap();

        let found = store.target_for_handle("+819000000000").unwrap().unwrap();
        assert_eq!(found.display_name, "母");
        assert_eq!(found.handles.len(), 2);
        assert!(store.target_for_handle("stranger@example.com").unwrap().is_none());
    }
}
