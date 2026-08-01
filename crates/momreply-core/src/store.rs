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
use crate::questions::{Question, QuestionKind};

/// `read_pending_row` が期待する列順。
const PENDING_COLUMNS: &str =
    "SELECT id, question, context, kind, answer, chat_rowid, created_at FROM pending_questions";

/// スキーマバージョン。`PRAGMA user_version` で管理する。
const SCHEMA_VERSION: i32 = 3;

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
  question    TEXT    NOT NULL,      -- 原文のまま
  context     TEXT,                  -- 質問の前に置かれた状況説明
  kind        TEXT    NOT NULL DEFAULT 'fact',  -- fact|visit
  norm_key    TEXT    NOT NULL DEFAULT '',  -- 表記ゆれを吸収した重複判定キー
  answer      TEXT,
  answered_at INTEGER,
  created_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_pending_unanswered
  ON pending_questions(target_id, answered_at);
-- norm_key のインデックスは migrate() 側で張る。
-- v1 の app.db には列が無く、ALTER TABLE より先にここを実行すると落ちる。

-- 定型回答。その都度変わるように見えて実際は一貫している質問
-- （「明日来る？」など）に、既定の答えを 1 つ持たせる。
-- self.md の「事実」にはできない種類なので分けている。
CREATE TABLE IF NOT EXISTS standing_answers (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  target_id    INTEGER NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
  kind         TEXT    NOT NULL,     -- questions::QuestionKind に対応
  answer       TEXT    NOT NULL,
  -- 初回のみ確認: この定型回答で自動送信する前に一度だけ人間の承認が要る。
  -- NULL の間は自動送信せず awaiting_review に倒す。
  confirmed_at INTEGER,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  UNIQUE(target_id, kind)
);

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

/// 答える材料がまだ無い質問。
#[derive(Debug, Clone)]
pub struct PendingQuestion {
    pub id: i64,
    /// 原文のまま。人間に見せるのはこちら。
    pub question: String,
    /// 質問の前に置かれた状況説明。
    pub context: Option<String>,
    pub kind: QuestionKind,
    pub answer: Option<String>,
    pub chat_rowid: Option<i64>,
    pub created_at: i64,
}

/// 定型回答。
#[derive(Debug, Clone)]
pub struct StandingAnswer {
    pub id: i64,
    pub kind: QuestionKind,
    pub answer: String,
    /// `None` の間は自動送信に使わない（初回のみ確認）。
    pub confirmed_at: Option<i64>,
}

impl StandingAnswer {
    /// 自動送信に使ってよいか。
    pub fn is_confirmed(&self) -> bool {
        self.confirmed_at.is_some()
    }
}

fn kind_label(kind: QuestionKind) -> &'static str {
    match kind {
        QuestionKind::Visit => "visit",
        QuestionKind::Fact => "fact",
    }
}

fn kind_from_label(label: &str) -> QuestionKind {
    match label {
        "visit" => QuestionKind::Visit,
        _ => QuestionKind::Fact,
    }
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

        // v1 で作られた app.db には norm_key が無い。CREATE TABLE IF NOT EXISTS
        // では追加されないので明示的に足す。インデックスは列が揃ってから張る。
        if !self.has_column("pending_questions", "norm_key")? {
            self.conn.execute(
                "ALTER TABLE pending_questions ADD COLUMN norm_key TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
        // v3: 長文から切り出した状況説明と、質問の種類。
        if !self.has_column("pending_questions", "context")? {
            self.conn
                .execute("ALTER TABLE pending_questions ADD COLUMN context TEXT", [])?;
        }
        if !self.has_column("pending_questions", "kind")? {
            self.conn.execute(
                "ALTER TABLE pending_questions ADD COLUMN kind TEXT NOT NULL DEFAULT 'fact'",
                [],
            )?;
        }
        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_pending_key
               ON pending_questions(target_id, norm_key);",
        )?;

        self.conn
            .pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(())
    }

    fn has_column(&self, table: &str, column: &str) -> Result<bool> {
        let mut stmt = self
            .conn
            .prepare(&format!("PRAGMA table_info({table})"))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            if row.get::<_, String>(1)? == column {
                return Ok(true);
            }
        }
        Ok(false)
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

    // MARK: 未回答質問（self.md の材料集め）

    /// 抽出した質問を記録する。次のものは積まない。
    ///
    /// - 既に答えたもの・未回答で溜まっているもの（同じことを二度聞かないため）
    /// - 定型回答が用意されている種類のもの（人間に聞く必要が無いため）
    ///
    /// 戻り値は新しく積まれた質問の件数。
    pub fn record_questions(
        &self,
        target_id: i64,
        chat_rowid: i64,
        questions: &[Question],
    ) -> Result<usize> {
        let now = now_unix();
        let mut added = 0;
        for q in questions {
            let kind = q.kind();

            // 定型回答があるなら人間に聞かない。未確認でも「答えは決まって
            // いるが自動送信の承認がまだ」という状態なので、質問としては積まない。
            if self.standing_answer(target_id, kind)?.is_some() {
                continue;
            }

            let key = crate::questions::normalize(&q.text);
            if self.find_question_by_key(target_id, &key)?.is_some() {
                continue;
            }

            self.conn.execute(
                "INSERT INTO pending_questions
                   (target_id, chat_rowid, question, context, kind, norm_key, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                (
                    target_id,
                    chat_rowid,
                    &q.text,
                    &q.context,
                    kind_label(kind),
                    &key,
                    now,
                ),
            )?;
            added += 1;
        }
        Ok(added)
    }

    /// 表記ゆれを吸収したキーで既存の質問を引く。
    fn find_question_by_key(&self, target_id: i64, key: &str) -> Result<Option<PendingQuestion>> {
        self.conn
            .query_row(
                &format!("{PENDING_COLUMNS} WHERE target_id = ?1 AND norm_key = ?2 ORDER BY id LIMIT 1"),
                (target_id, key),
                Self::read_pending_row,
            )
            .optional()
            .map_err(Into::into)
    }

    // MARK: 定型回答

    /// 種類に対する定型回答を引く。
    pub fn standing_answer(
        &self,
        target_id: i64,
        kind: QuestionKind,
    ) -> Result<Option<StandingAnswer>> {
        self.conn
            .query_row(
                "SELECT id, kind, answer, confirmed_at
                 FROM standing_answers WHERE target_id = ?1 AND kind = ?2",
                (target_id, kind_label(kind)),
                |row| {
                    Ok(StandingAnswer {
                        id: row.get(0)?,
                        kind: kind_from_label(&row.get::<_, String>(1)?),
                        answer: row.get(2)?,
                        confirmed_at: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// 定型回答を設定する。
    ///
    /// **内容を変えたら確認状態はリセットされる。**
    /// 一度承認した文面のまま別のことを自動送信させないため。
    pub fn set_standing_answer(
        &self,
        target_id: i64,
        kind: QuestionKind,
        answer: &str,
    ) -> Result<StandingAnswer> {
        let now = now_unix();
        let existing = self.standing_answer(target_id, kind)?;
        let unchanged = existing.as_ref().is_some_and(|s| s.answer == answer);

        self.conn.execute(
            "INSERT INTO standing_answers (target_id, kind, answer, confirmed_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(target_id, kind) DO UPDATE SET
               answer = excluded.answer,
               confirmed_at = excluded.confirmed_at,
               updated_at = excluded.updated_at",
            (
                target_id,
                kind_label(kind),
                answer,
                // 文面が変わっていなければ確認状態を保つ。
                unchanged.then(|| existing.as_ref().and_then(|s| s.confirmed_at)).flatten(),
                now,
            ),
        )?;

        // その種類の未回答質問はもう人間に聞く必要が無い。
        // 残すと「答えるべきこと」の一覧が実態とずれる。
        self.conn.execute(
            "DELETE FROM pending_questions
             WHERE target_id = ?1 AND kind = ?2 AND answer IS NULL",
            (target_id, kind_label(kind)),
        )?;

        self.standing_answer(target_id, kind)?
            .context("定型回答を保存できたが読み出せない")
    }

    /// 定型回答を自動送信に使うことを承認する（初回のみ確認）。
    pub fn confirm_standing_answer(&self, target_id: i64, kind: QuestionKind) -> Result<()> {
        let updated = self.conn.execute(
            "UPDATE standing_answers SET confirmed_at = ?3, updated_at = ?3
             WHERE target_id = ?1 AND kind = ?2",
            (target_id, kind_label(kind), now_unix()),
        )?;
        if updated == 0 {
            bail!("承認する定型回答が無い。先に set で登録すること");
        }
        Ok(())
    }

    pub fn list_standing_answers(&self, target_id: i64) -> Result<Vec<StandingAnswer>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, answer, confirmed_at FROM standing_answers
             WHERE target_id = ?1 ORDER BY kind",
        )?;
        let rows = stmt.query_map([target_id], |row| {
            Ok(StandingAnswer {
                id: row.get(0)?,
                kind: kind_from_label(&row.get::<_, String>(1)?),
                answer: row.get(2)?,
                confirmed_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 既に答えを持っている質問なら、その答えを返す。
    ///
    /// 生成の前に呼ぶ。答えがあれば人間に聞かずに生成へ進める。
    pub fn known_answer(&self, target_id: i64, question: &str) -> Result<Option<String>> {
        let key = crate::questions::normalize(question);
        Ok(self
            .find_question_by_key(target_id, &key)?
            .and_then(|q| q.answer))
    }

    pub fn unanswered_questions(&self, target_id: i64) -> Result<Vec<PendingQuestion>> {
        let mut stmt = self.conn.prepare(&format!(
            "{PENDING_COLUMNS} WHERE target_id = ?1 AND answer IS NULL ORDER BY id"
        ))?;
        let rows = stmt.query_map([target_id], Self::read_pending_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 質問に答える。app.db に記録し、`self.md` にも事実として追記する。
    pub fn answer_question(&self, question_id: i64, answer: &str) -> Result<PendingQuestion> {
        let q = self
            .conn
            .query_row(
                &format!("{PENDING_COLUMNS} WHERE id = ?1"),
                [question_id],
                Self::read_pending_row,
            )
            .optional()?
            .with_context(|| format!("質問 #{question_id} が無い"))?;

        self.conn.execute(
            "UPDATE pending_questions SET answer = ?2, answered_at = ?3 WHERE id = ?1",
            (question_id, answer, now_unix()),
        )?;

        Ok(PendingQuestion {
            answer: Some(answer.to_string()),
            ..q
        })
    }

    // MARK: kv

    pub fn get_kv(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .map_err(Into::into)
    }

    pub fn set_kv(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO kv (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            (key, value),
        )?;
        Ok(())
    }

    fn handles_of(&self, target_id: i64) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT chat_identifier FROM target_handles WHERE target_id = ?1 ORDER BY chat_identifier",
        )?;
        let rows = stmt.query_map([target_id], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn read_pending_row(row: &rusqlite::Row) -> rusqlite::Result<PendingQuestion> {
        Ok(PendingQuestion {
            id: row.get(0)?,
            question: row.get(1)?,
            context: row.get(2)?,
            kind: kind_from_label(&row.get::<_, String>(3)?),
            answer: row.get(4)?,
            chat_rowid: row.get(5)?,
            created_at: row.get(6)?,
        })
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

    // MARK: 定型回答（初回のみ確認）

    fn store_with_target() -> (Store, i64) {
        let chat_db = fake_chat_db("t@example.com", 3);
        let mut store = Store::open_in_memory().unwrap();
        let t = store
            .add_target(&chat_db, new_target("t", "t@example.com"))
            .unwrap();
        (store, t.id)
    }

    /// 設定しただけでは自動送信に使わせない。
    #[test]
    fn a_new_standing_answer_starts_unconfirmed() {
        let (store, id) = store_with_target();
        let saved = store
            .set_standing_answer(id, QuestionKind::Visit, "行かない")
            .unwrap();
        assert!(!saved.is_confirmed());
    }

    #[test]
    fn confirming_enables_it() {
        let (store, id) = store_with_target();
        store
            .set_standing_answer(id, QuestionKind::Visit, "行かない")
            .unwrap();
        store.confirm_standing_answer(id, QuestionKind::Visit).unwrap();
        assert!(store
            .standing_answer(id, QuestionKind::Visit)
            .unwrap()
            .unwrap()
            .is_confirmed());
    }

    /// 承認済みの文面を書き換えたら、承認は取り消す。
    /// でないと「確認した文面」と「実際に送る文面」がずれる。
    #[test]
    fn changing_the_text_revokes_confirmation() {
        let (store, id) = store_with_target();
        store
            .set_standing_answer(id, QuestionKind::Visit, "行かない")
            .unwrap();
        store.confirm_standing_answer(id, QuestionKind::Visit).unwrap();

        let changed = store
            .set_standing_answer(id, QuestionKind::Visit, "その日は行く")
            .unwrap();
        assert!(
            !changed.is_confirmed(),
            "文面を変えたのに承認が残っている"
        );
    }

    /// 同じ文面で保存し直しただけなら承認は維持する。
    #[test]
    fn resaving_the_same_text_keeps_confirmation() {
        let (store, id) = store_with_target();
        store
            .set_standing_answer(id, QuestionKind::Visit, "行かない")
            .unwrap();
        store.confirm_standing_answer(id, QuestionKind::Visit).unwrap();

        let again = store
            .set_standing_answer(id, QuestionKind::Visit, "行かない")
            .unwrap();
        assert!(again.is_confirmed());
    }

    #[test]
    fn cannot_confirm_what_was_never_set() {
        let (store, id) = store_with_target();
        assert!(store.confirm_standing_answer(id, QuestionKind::Visit).is_err());
    }

    /// 定型回答があれば、その種類の質問で人間を呼び出さない。
    #[test]
    fn a_standing_answer_stops_questions_from_piling_up() {
        let (store, id) = store_with_target();
        let visit = Question {
            text: "明日来る？".into(),
            context: None,
        };
        assert_eq!(visit.kind(), QuestionKind::Visit);

        assert_eq!(store.record_questions(id, 1, &[visit.clone()]).unwrap(), 1);

        // 定型回答を入れると、以後は積まれない。
        store
            .set_standing_answer(id, QuestionKind::Visit, "行かない")
            .unwrap();
        assert_eq!(store.record_questions(id, 2, &[visit]).unwrap(), 0);

        // 既に溜まっていた分も掃除される。人間に聞く必要が無くなったため。
        assert!(store.unanswered_questions(id).unwrap().is_empty());
    }

    /// 事実型は定型回答の対象外。材料が無ければ人間に聞く。
    #[test]
    fn factual_questions_still_reach_the_human() {
        let (store, id) = store_with_target();
        store
            .set_standing_answer(id, QuestionKind::Visit, "行かない")
            .unwrap();

        let fact = Question {
            text: "保険証はありますか？".into(),
            context: None,
        };
        assert_eq!(store.record_questions(id, 1, &[fact]).unwrap(), 1);
        assert_eq!(store.unanswered_questions(id).unwrap().len(), 1);
    }

    #[test]
    fn the_same_question_is_never_asked_twice() {
        let (store, id) = store_with_target();
        let q = Question {
            text: "保険証は、ありますか？".into(),
            context: None,
        };
        assert_eq!(store.record_questions(id, 1, &[q.clone()]).unwrap(), 1);
        assert_eq!(store.record_questions(id, 2, &[q]).unwrap(), 0);
    }

    #[test]
    fn context_survives_the_round_trip() {
        let (store, id) = store_with_target();
        store
            .record_questions(
                id,
                1,
                &[Question {
                    text: "くる？".into(),
                    context: Some("今日バーベキューをします".into()),
                }],
            )
            .unwrap();
        let pending = store.unanswered_questions(id).unwrap();
        assert_eq!(pending[0].context.as_deref(), Some("今日バーベキューをします"));
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
