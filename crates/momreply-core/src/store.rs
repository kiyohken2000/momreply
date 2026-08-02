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
const SCHEMA_VERSION: i32 = 7;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS targets (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  slug          TEXT    NOT NULL UNIQUE,          -- プロファイルのファイル名に使う
  display_name  TEXT    NOT NULL,
  enabled       INTEGER NOT NULL DEFAULT 1,
  -- 既定は OFF。配布時に自動送信が既定で走る状態にしてはいけない。
  auto_send     INTEGER NOT NULL DEFAULT 0,
  -- 'mirror' などのプリセット名か、'chars:400' の形の目標文字数
  -- （[`crate::pipeline::LengthPreset`]）。
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
  length_preset_override TEXT,
  -- レート制限をここから数え直す（仕様書には無い）。
  -- 履歴そのものは消さない。消すと何を送ったかの記録まで失われる。
  rate_reset_at          INTEGER
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

-- self.md への追記候補。**承認するまで反映しない。**
-- self.md は AI が事実として断定する唯一の材料なので、誤りが入ると
-- 以後すべての生成が汚染される（仕様書 6.7 と同じ理由）。
CREATE TABLE IF NOT EXISTS fact_candidates (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  section       TEXT    NOT NULL,           -- 事実|答えたくないこと|伝え方
  content       TEXT    NOT NULL,
  -- 根拠になったやり取り。人が正しさを判断するために必ず持たせる。
  evidence_ask  TEXT,
  evidence_reply TEXT,
  source_rowid  INTEGER,
  source_chat   TEXT,
  confidence    TEXT    NOT NULL DEFAULT 'medium',
  status        TEXT    NOT NULL DEFAULT 'pending',  -- pending|approved|rejected
  created_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_fact_status ON fact_candidates(status);

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
    /// プリセット名か `chars:400`。[`crate::pipeline::LengthPreset`] が読む。
    pub reply_preset: String,
    pub handles: Vec<String>,
    pub last_seen_rowid: Option<i64>,
}

/// `generation_log` に残す 1 件。
pub struct GenerationRecord<'a> {
    pub target_id: i64,
    pub chat_rowid: i64,
    /// `initial` | `regenerate` | `profile_update` | `classification`
    pub kind: &'a str,
    pub provider: &'a str,
    pub model: &'a str,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub latency_ms: u64,
    /// 再生成時のユーザー指示。何を直させたかを後から追えるようにする。
    pub user_instruction: Option<&'a str>,
    pub output: Option<&'a str>,
    pub error: Option<&'a str>,
}

/// 人の確認を待っている 1 件。
#[derive(Debug, Clone)]
pub struct PendingItem {
    pub chat_rowid: i64,
    pub target_id: i64,
    pub target_slug: String,
    pub display_name: String,
    /// 受信元。**返信はここに送る**（仕様書 6.3）。
    pub chat_guid: String,
    pub received_at: i64,
    pub body: Option<String>,
    pub status: String,
    pub skip_reason: Option<String>,
    pub draft: Option<String>,
}

/// `self.md` への追記候補。**承認するまで反映しない。**
#[derive(Debug, Clone)]
pub struct FactCandidate {
    pub id: i64,
    pub section: String,
    pub content: String,
    /// 根拠になったやり取り。人が正しさを判断するのに要る。
    pub evidence_ask: Option<String>,
    pub evidence_reply: Option<String>,
    pub source_rowid: Option<i64>,
    pub source_chat: Option<String>,
    pub confidence: String,
}

/// 相手ごとの実行状態。
#[derive(Debug, Clone)]
pub struct TargetRuntime {
    pub last_seen_rowid: Option<i64>,
    pub consecutive_auto: u32,
    pub last_sent_at: Option<i64>,
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

    fn has_column(&self, table: &str, column: &str) -> Result<bool> {
        let mut stmt = self.conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            if row.get::<_, String>(1)? == column {
                return Ok(true);
            }
        }
        Ok(false)
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

        // v7: レート制限の数え直し。
        if !self.has_column("target_state", "rate_reset_at")? {
            self.conn
                .execute("ALTER TABLE target_state ADD COLUMN rate_reset_at INTEGER", [])?;
        }

        // v6: 質問を人に聞く仕組みをやめた。生成は常に「おまかせ」で、
        // 確定的な答えを出さないので、聞くべきことがそもそも出ない。
        // 既に作られている app.db では、この 2 つのテーブルは孤児になる。
        // 中身は個人的なやり取りなので、残さず落とす。
        self.conn.execute_batch(
            "DROP TABLE IF EXISTS pending_questions;
             DROP TABLE IF EXISTS standing_answers;",
        )?;

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

    pub fn set_reply_preset(&self, target_id: i64, preset: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE targets SET reply_preset = ?2, updated_at = ?3 WHERE id = ?1",
            (target_id, preset, now_unix()),
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

    // MARK: few-shot

    /// few-shot を入れ替える。古い分は捨てる。
    pub fn replace_fewshot(&self, target_id: i64, pairs: &[crate::fewshot::Pair]) -> Result<()> {
        let now = now_unix();
        self.conn.execute(
            "DELETE FROM fewshot_pairs WHERE target_id = ?1",
            [target_id],
        )?;
        for p in pairs {
            self.conn.execute(
                "INSERT INTO fewshot_pairs (target_id, incoming, reply, source_rowid, built_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                (target_id, &p.incoming, &p.reply, p.source_rowid, now),
            )?;
        }
        Ok(())
    }

    pub fn fewshot(&self, target_id: i64) -> Result<Vec<crate::fewshot::Pair>> {
        let mut stmt = self.conn.prepare(
            "SELECT incoming, reply, source_rowid FROM fewshot_pairs
             WHERE target_id = ?1 ORDER BY source_rowid",
        )?;
        let rows = stmt.query_map([target_id], |row| {
            Ok(crate::fewshot::Pair {
                incoming: row.get(0)?,
                reply: row.get(1)?,
                source_rowid: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // MARK: 記録

    pub fn log_generation(&self, rec: &GenerationRecord<'_>) -> Result<()> {
        self.conn.execute(
            "INSERT INTO generation_log
               (target_id, chat_rowid, kind, provider, model, input_tokens, output_tokens,
                latency_ms, user_instruction, output, error, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            (
                rec.target_id,
                rec.chat_rowid,
                rec.kind,
                rec.provider,
                rec.model,
                rec.input_tokens,
                rec.output_tokens,
                rec.latency_ms as i64,
                rec.user_instruction,
                rec.output,
                rec.error,
                now_unix(),
            ),
        )?;
        Ok(())
    }

    /// 前回の生成結果。再生成の入力に使う。
    pub fn previous_draft(&self, chat_rowid: i64) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT draft FROM processed_messages WHERE chat_rowid = ?1",
                [chat_rowid],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .map(Option::flatten)
            .map_err(Into::into)
    }

    /// 処理結果を記録する。`status` は仕様書 5.2 の値。
    #[allow(clippy::too_many_arguments)]
    pub fn record_processed(
        &self,
        target_id: i64,
        chat_rowid: i64,
        chat_guid: &str,
        received_at: i64,
        body: Option<&str>,
        status: &str,
        skip_reason: Option<&str>,
        draft: Option<&str>,
        provider: Option<&str>,
        model: Option<&str>,
    ) -> Result<()> {
        let now = now_unix();
        self.conn.execute(
            "INSERT INTO processed_messages
               (chat_rowid, target_id, chat_guid, received_at, body, status,
                skip_reason, draft, provider, model, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11)
             ON CONFLICT(chat_rowid) DO UPDATE SET
               status = excluded.status,
               skip_reason = excluded.skip_reason,
               draft = excluded.draft,
               provider = excluded.provider,
               model = excluded.model,
               updated_at = excluded.updated_at",
            (
                chat_rowid,
                target_id,
                chat_guid,
                received_at,
                body,
                status,
                skip_reason,
                draft,
                provider,
                model,
                now,
            ),
        )?;
        Ok(())
    }

    // MARK: self.md への追記候補

    /// 候補を積む。同じ内容が既にあれば積まない。
    pub fn add_fact_candidate(&self, c: &FactCandidate) -> Result<bool> {
        let exists: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM fact_candidates WHERE content = ?1 AND status != 'rejected'",
            [&c.content],
            |r| r.get(0),
        )?;
        if exists > 0 {
            return Ok(false);
        }
        self.conn.execute(
            "INSERT INTO fact_candidates
               (section, content, evidence_ask, evidence_reply, source_rowid,
                source_chat, confidence, status, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,'pending',?8)",
            (
                &c.section,
                &c.content,
                &c.evidence_ask,
                &c.evidence_reply,
                c.source_rowid,
                &c.source_chat,
                &c.confidence,
                now_unix(),
            ),
        )?;
        Ok(true)
    }

    pub fn pending_facts(&self) -> Result<Vec<FactCandidate>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, section, content, evidence_ask, evidence_reply,
                    source_rowid, source_chat, confidence
             FROM fact_candidates WHERE status = 'pending' ORDER BY confidence DESC, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(FactCandidate {
                id: r.get(0)?,
                section: r.get(1)?,
                content: r.get(2)?,
                evidence_ask: r.get(3)?,
                evidence_reply: r.get(4)?,
                source_rowid: r.get(5)?,
                source_chat: r.get(6)?,
                confidence: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn fact_candidate(&self, id: i64) -> Result<Option<FactCandidate>> {
        self.conn
            .query_row(
                "SELECT id, section, content, evidence_ask, evidence_reply,
                        source_rowid, source_chat, confidence
                 FROM fact_candidates WHERE id = ?1",
                [id],
                |r| {
                    Ok(FactCandidate {
                        id: r.get(0)?,
                        section: r.get(1)?,
                        content: r.get(2)?,
                        evidence_ask: r.get(3)?,
                        evidence_reply: r.get(4)?,
                        source_rowid: r.get(5)?,
                        source_chat: r.get(6)?,
                        confidence: r.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn set_fact_status(&self, id: i64, status: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE fact_candidates SET status = ?2 WHERE id = ?1",
            (id, status),
        )?;
        Ok(())
    }

    // MARK: 確認待ちの一覧

    /// 人の確認を待っているもの（仕様書 6.6）。新しい順。
    pub fn pending_items(&self, limit: u32) -> Result<Vec<PendingItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.chat_rowid, p.target_id, t.slug, t.display_name, p.chat_guid,
                    p.received_at, p.body, p.status, p.skip_reason, p.draft
             FROM processed_messages p
             JOIN targets t ON t.id = p.target_id
             WHERE p.status IN ('awaiting_review', 'dry_run')
             ORDER BY p.received_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |r| {
            Ok(PendingItem {
                chat_rowid: r.get(0)?,
                target_id: r.get(1)?,
                target_slug: r.get(2)?,
                display_name: r.get(3)?,
                chat_guid: r.get(4)?,
                received_at: r.get(5)?,
                body: r.get(6)?,
                status: r.get(7)?,
                skip_reason: r.get(8)?,
                draft: r.get(9)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn mark_skipped(&self, chat_rowid: i64, reason: &str) -> Result<()> {
        let now = now_unix();
        self.conn.execute(
            "UPDATE processed_messages
             SET status = 'skipped', skip_reason = ?2, updated_at = ?3
             WHERE chat_rowid = ?1",
            (chat_rowid, reason, now),
        )?;
        Ok(())
    }

    // MARK: ガードに渡す集計

    /// 直近 `secs` 秒に自動送信した件数。
    /// 直近 `secs` 秒に自動送信した件数。
    ///
    /// `rate_reset_at` より前の送信は数えない。**履歴は残したまま、
    /// 数え直しの起点だけ動かす。** 履歴を消すと、何を送ったかの記録まで
    /// 失われる。
    pub fn sent_within(&self, target_id: i64, secs: i64) -> Result<u32> {
        let since = now_unix() - secs;
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM processed_messages p
             JOIN target_state s ON s.target_id = p.target_id
             WHERE p.target_id = ?1 AND p.status = 'sent'
               AND p.sent_at >= ?2
               AND p.sent_at > COALESCE(s.rate_reset_at, 0)",
            (target_id, since),
            |r| r.get(0),
        )?;
        Ok(n as u32)
    }

    /// レート制限を数え直す起点を「いま」にする。
    ///
    /// 連続カウントと違い、これは**実際に送った件数の歯止めを外す**。
    /// 押した人が意図して外したことになるので、記録は残す。
    pub fn reset_rate_window(&self, target_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE target_state SET rate_reset_at = ?2 WHERE target_id = ?1",
            (target_id, now_unix()),
        )?;
        Ok(())
    }

    /// 当月の推定コスト（USD）。
    ///
    /// 単価は kv に `pricing.<provider>:<model>.input` / `.output` の形で
    /// 100 万トークンあたりの USD で入れる。**単価が無いモデルは 0 として
    /// 扱う**（仕様書 6.4.5.2）。0 のまま上限に達しないのは想定どおりで、
    /// UI 側で「単価未設定」と示す必要がある。
    pub fn month_cost_usd(&self, target_id: i64) -> Result<f64> {
        use chrono::{Datelike, TimeZone};
        let month_start = chrono::Local::now()
            .date_naive()
            .with_day(1)
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .and_then(|dt| chrono::Local.from_local_datetime(&dt).single())
            .map(|dt| dt.timestamp())
            .unwrap_or(0);

        let mut stmt = self.conn.prepare(
            "SELECT provider, model, SUM(COALESCE(input_tokens,0)), SUM(COALESCE(output_tokens,0))
             FROM generation_log
             WHERE target_id = ?1 AND created_at >= ?2 AND provider IS NOT NULL
             GROUP BY provider, model",
        )?;
        let rows = stmt.query_map((target_id, month_start), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;

        let mut total = 0.0;
        for row in rows {
            let (provider, model, input, output) = row?;
            let key = format!("pricing.{provider}:{model}");
            let unit_in = self.pricing(&format!("{key}.input"))?;
            let unit_out = self.pricing(&format!("{key}.output"))?;
            total += (input as f64 / 1_000_000.0) * unit_in;
            total += (output as f64 / 1_000_000.0) * unit_out;
        }
        Ok(total)
    }

    fn pricing(&self, key: &str) -> Result<f64> {
        Ok(self
            .get_kv(key)?
            .and_then(|v| v.trim().parse::<f64>().ok())
            .unwrap_or(0.0))
    }

    /// 相手ごとの実行状態。
    pub fn target_runtime(&self, target_id: i64) -> Result<TargetRuntime> {
        self.conn
            .query_row(
                "SELECT last_seen_rowid, consecutive_auto_count, last_sent_at
                 FROM target_state WHERE target_id = ?1",
                [target_id],
                |r| {
                    Ok(TargetRuntime {
                        last_seen_rowid: r.get(0)?,
                        consecutive_auto: r.get::<_, i64>(1)? as u32,
                        last_sent_at: r.get(2)?,
                    })
                },
            )
            .optional()?
            .context("target_state が無い。ターゲット登録が壊れている")
    }

    /// 自動送信に成功したときに呼ぶ（仕様書 6.4.5.1）。
    pub fn note_auto_sent(&self, target_id: i64, at: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE target_state
             SET consecutive_auto_count = consecutive_auto_count + 1, last_sent_at = ?2
             WHERE target_id = ?1",
            (target_id, at),
        )?;
        Ok(())
    }

    /// 人が介入したときに呼ぶ。連続カウンタを 0 に戻す（仕様書 6.4.5.1）。
    pub fn reset_consecutive(&self, target_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE target_state SET consecutive_auto_count = 0 WHERE target_id = ?1",
            [target_id],
        )?;
        Ok(())
    }

    /// 送信できたことを記録する。
    pub fn mark_sent(&self, chat_rowid: i64, final_text: &str, sent_rowid: Option<i64>) -> Result<()> {
        let now = now_unix();
        self.conn.execute(
            "UPDATE processed_messages
             SET status = 'sent', final_text = ?2, sent_at = ?3, sent_rowid = ?4, updated_at = ?3
             WHERE chat_rowid = ?1",
            (chat_rowid, final_text, now, sent_rowid),
        )?;
        Ok(())
    }

    pub fn mark_failed(&self, chat_rowid: i64, reason: &str) -> Result<()> {
        let now = now_unix();
        self.conn.execute(
            "UPDATE processed_messages
             SET status = 'failed', skip_reason = ?2, updated_at = ?3
             WHERE chat_rowid = ?1",
            (chat_rowid, reason, now),
        )?;
        Ok(())
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

    /// 数え直しても、送信履歴そのものは残る。
    /// 消してしまうと、何を送ったかの記録まで失われる。
    #[test]
    fn resetting_the_rate_window_keeps_the_history() {
        let chat_db = fake_chat_db("r@example.com", 3);
        let mut store = Store::open_in_memory().unwrap();
        let t = store
            .add_target(&chat_db, new_target("r", "r@example.com"))
            .unwrap();

        for rowid in 1..=3 {
            store
                .record_processed(
                    t.id,
                    rowid,
                    "r@example.com",
                    now_unix(),
                    Some("やあ"),
                    "sent",
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
            store.mark_sent(rowid, "うん", Some(rowid + 100)).unwrap();
        }
        assert_eq!(store.sent_within(t.id, 3600).unwrap(), 3);

        store.reset_rate_window(t.id).unwrap();
        assert_eq!(store.sent_within(t.id, 3600).unwrap(), 0);

        // 履歴は残っている。
        let n: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM processed_messages WHERE status = 'sent'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 3);
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
