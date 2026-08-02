# MomReply — iMessage 自動返信アプリ 仕様書

**バージョン**: 1.0
**対象実装**: Claude Code
**作成日**: 2026-08-01

> **この文書は最初の設計書です。実装は途中で意図的に離れました。**
> コード中の「仕様書 6.4」のような参照はこの文書を指しますが、
> 次の点は現在の実装と食い違います。実装のほうが新しい判断です。
>
> | この文書 | 現在の実装 |
> |---|---|
> | 監視対象は母 1 人に固定 | 相手は任意に選べる（`targets` テーブル） |
> | 質問には具体的に答える。材料が無ければ人に聞く | 具体的な答えは出さない。人には聞かない |
> | 曖昧なら「確認してみる」と返す | 確定させないが、そっけなくもしない |
> | `NSWorkspace.didWakeNotification` を購読 | 前回ポーリングからの経過時間で判定 |
>
> 離れた理由は、それぞれの実装のドキュメンテーションコメントに書いてあります。

### 名称と識別子

| 項目 | 値 |
|---|---|
| アプリ表示名 | MomReply |
| bundle identifier | `net.votepurchase.momreply` |
| 実行バイナリ名 | `momreply` |
| Swift サイドカー | `momreply-fm` |
| Tauri プロダクト名 | `MomReply` |
| Keychain service | `net.votepurchase.momreply` |
| 設定ファイル | `~/Library/Application Support/net.votepurchase.momreply/config.toml` |
| ローカル DB | `~/Library/Application Support/net.votepurchase.momreply/app.db` |
| プロファイル | `~/Library/Application Support/net.votepurchase.momreply/profile.md` |
| ログ | `~/Library/Logs/net.votepurchase.momreply/app.log` |

フルディスクアクセスとオートメーション権限は bundle identifier 単位で付与される。**この値は変更しないこと。**変更すると付与済みの権限がすべて無効になり、システム設定から古いエントリを削除して付け直す必要がある。

---

## 1. 概要

macOS 上で常駐し、特定の相手（母）から届く iMessage を検知して、LLM が返信文を生成し、自動送信または手動確認のうえ送信するメニューバーアプリ。

### 1.1 目的

- 母からのメッセージへの返信を、実質的な手間ゼロで維持する
- 生成される文章が「AIっぽい」ではなく「自分が書いたもの」に見えること
- 誤爆したときに気づけること

### 1.2 設計上の最重要方針

1. **chat.db には絶対に書き込まない**（read-only 接続のみ）
2. **監視対象は母のハンドルのみ**。他の相手のメッセージは読み込みすらしない
3. **失敗時は沈黙する**。API エラー時に定型文を送るような挙動は実装しない
4. **すべての生成・送信をログに残す**

---

## 2. スコープ

### 2.1 v1 に含む

- 母からのメッセージ検知（chat.db ポーリング）
- LLM による返信文生成（単一案）
- 全自動送信モード
- メニューバーのポップオーバーによる手動確認・編集・送信
- 母プロファイル（メモ）の AI 自動更新 + 手動修正
- 自分の過去返信を few-shot として抽出・利用
- LLM プロバイダの切り替え（Claude / Gemini / OpenAI）
- ドライランモード
- 送信結果の検証とログ

### 2.2 v1 に含まない

- 複数トーンの案を同時生成（採用しない。単一案 + 直接編集が主動線）
- 母以外の相手への対応（設計上は拡張可能にするが UI は出さない）
- 画像・添付ファイルの送信
- グループチャット対応
- iOS アプリ / リモート操作

---

## 3. 技術スタック

| 領域 | 採用 | 理由 |
|---|---|---|
| フレームワーク | Tauri v2 | 署名済み .app として配布でき、フルディスクアクセスの付与が1回で済む。トレイ + ポップオーバーが標準機能 |
| コア | Rust | `imessage-database` クレートで chat.db の attributedBody を確実にデコードできる |
| UI | TypeScript + React + Tailwind CSS | 後から自分で調整しやすい |
| ローカル LLM | Swift サイドカー（Foundation Models framework） | Apple Intelligence をオンデバイスで呼ぶ。Swift API のため Rust から直接叩けず、小さな CLI を同梱する（7.3 参照） |
| ローカル DB | SQLite（`rusqlite`） | アプリ状態・ログ・キャッシュ |
| APIキー保管 | macOS Keychain（`keyring` クレート） | 平文保存しない |

### 3.1 主要クレート

```toml
[dependencies]
tauri = { version = "2", features = ["tray-icon"] }
tauri-plugin-notification = "2"
tauri-plugin-autostart = "2"
tauri-plugin-positioner = "2"   # トレイ直下へのウィンドウ配置
imessage-database = "*"          # chat.db 読み取り + typedstream デコード
rusqlite = { version = "0.32", features = ["bundled"] }
reqwest = { version = "0.12", features = ["json", "rustls-tls"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
keyring = "3"
notify = "7"                     # chat.db-wal の FSEvents 監視
chrono = "0.4"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
```

**注意**: `imessage-database` の API はバージョンによって変わる。実装開始時に `cargo doc --open` で現行シグネチャを確認すること。特に `util::streamtyped::parse` と `tables::messages::Message` 周辺。

---

## 4. アーキテクチャ

```
┌─────────────────────────────────────────────────┐
│ Tauri App (メニューバー常駐)                      │
│                                                 │
│  ┌──────────────┐      ┌────────────────────┐   │
│  │  UI (React)  │◄────►│  Tauri Commands    │   │
│  │  ポップオーバー│      │  (IPC bridge)      │   │
│  └──────────────┘      └─────────┬──────────┘   │
│                                   │              │
│  ┌────────────────────────────────▼───────────┐ │
│  │            Core (Rust)                      │ │
│  │  ┌──────────┐  ┌──────────┐  ┌───────────┐ │ │
│  │  │ Watcher  │─►│ Pipeline │─►│  Sender   │ │ │
│  │  └────┬─────┘  └────┬─────┘  └─────┬─────┘ │ │
│  │       │             │              │       │ │
│  │       │        ┌────▼─────┐        │       │ │
│  │       │        │   LLM    │        │       │ │
│  │       │        │ Provider │        │       │ │
│  │       │        └──────────┘        │       │ │
│  │  ┌────▼─────────────────────────────▼────┐ │ │
│  │  │  Store (app.db) / Profile / FewShot   │ │ │
│  │  └───────────────────────────────────────┘ │ │
│  └─────────────────────────────────────────────┘ │
└─────────────────┬──────────────┬────────────────┘
                  │              │
        read-only │              │ osascript
                  ▼              ▼
            ~/Library/       Messages.app
            Messages/
            chat.db
```

### 4.1 モジュール構成

```
src-tauri/src/
├── main.rs
├── config.rs          # 設定の読み書き
├── store.rs           # app.db スキーマ・CRUD
├── imessage/
│   ├── reader.rs      # chat.db 読み取り
│   ├── watcher.rs     # ポーリング + FSEvents + スリープ復帰検知
│   └── sender.rs      # osascript 送信 + 成否検証
├── llm/
│   ├── mod.rs         # LlmProvider trait
│   ├── anthropic.rs
│   ├── gemini.rs
│   └── openai.rs
├── pipeline/
│   ├── mod.rs         # 受信→生成→送信のオーケストレーション
│   ├── guards.rs      # セーフティガード群
│   └── prompt.rs      # プロンプト組み立て
├── profile.rs         # 母プロファイルの読み書き・AI更新
├── fewshot.rs         # 過去返信の抽出・キャッシュ
└── commands.rs        # Tauri コマンド（UI ↔ Core）
```

---

## 5. データモデル

### 5.1 chat.db（読み取り専用）

パス: `~/Library/Messages/chat.db`

**接続は必ず read-only**:

```rust
let conn = Connection::open_with_flags(
    path,
    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
)?;
```

コピーして読む方式を取る場合は `chat.db-wal` と `chat.db-shm` も必ず同時にコピーすること。片方だけだと直近のメッセージが欠落する。

#### 新着メッセージ取得クエリ

`handle` テーブル経由ではなく `chat` 経由で取得する。送信メッセージは `handle_id = 0` になることがあり、handle join だと自分の返信を取りこぼすため。

```sql
SELECT
  m.ROWID              AS rowid,
  m.guid,
  m.text,
  m.attributedBody,
  m.date,
  m.is_from_me,
  m.item_type,
  m.associated_message_type,
  m.associated_message_guid,
  m.balloon_bundle_id,
  c.chat_identifier
FROM message m
JOIN chat_message_join cmj ON cmj.message_id = m.ROWID
JOIN chat c               ON c.ROWID = cmj.chat_id
WHERE c.chat_identifier IN (:handles)
  AND m.ROWID > :last_seen_rowid
ORDER BY m.ROWID ASC;
```

#### 日時変換

`message.date` は Apple epoch（2001-01-01 00:00:00 UTC）からのナノ秒。

```sql
datetime(m.date / 1000000000 + strftime('%s','2001-01-01'), 'unixepoch', 'localtime')
```

#### 本文の取り出し

**`m.text` は NULL であることが常態**（macOS Ventura 以降、特に macOS 26 Tahoe）。以下の優先順で取得する。

1. `imessage_database::util::streamtyped::parse(attributedBody)` でデコード
2. 失敗したら `m.text` にフォールバック
3. 両方空なら「本文なしメッセージ」として扱い、生成対象から除外

デコード結果には以下の制御文字が含まれるので除去する:

- `U+FFFC` (OBJECT REPLACEMENT CHARACTER): 添付ファイル1件につき1つ
- `U+FFFD` (REPLACEMENT CHARACTER): App メッセージ

#### 除外すべき行

| 条件 | 意味 | 扱い |
|---|---|---|
| `is_from_me = 1` | 自分の送信 | トリガーにしない（既返信チェックには使う） |
| `associated_message_type != 0` | タップバック・リアクション | 無視 |
| `item_type != 0` | 参加者追加などのシステムメッセージ | 無視 |
| `balloon_bundle_id IS NOT NULL` | ステッカー・App メッセージ | 無視 |
| 本文が空 | 添付のみ | 生成せず通知のみ |
| `date_retracted IS NOT NULL AND date_retracted != 0` | 母が送信取り消しした | 無視。処理済みなら 5.1.1 参照 |

#### 5.1.1 送信取り消し・編集への対応

母が Undo Send（送信取り消し）や編集を使った場合、こちらは既に読み込んで返信を生成・送信している可能性がある。

**取得時のカラム追加**

```sql
m.date_retracted,
m.date_edited,
m.message_summary_info
```

**判定ルール**

| 状況 | 挙動 |
|---|---|
| 生成前に `date_retracted` が立っている | 処理せず `skip_reason = 'retracted'` で記録 |
| 生成中／送信待ちの間に取り消された | 送信を中止し `skip_reason = 'retracted'` に変更。通知「相手がメッセージを取り消したため送信を中止しました」 |
| 送信後に取り消された | 何もしない（既に送ってしまっている）。ログにフラグを立て、UI の履歴に「相手が取り消し済み」と表示 |
| 生成前に `date_edited` が立っている | 編集後の本文で処理する（`attributedBody` は編集後の内容に更新される） |
| 送信後に編集された | 通知のみ。自動で追加返信はしない |

**送信直前チェック**に取り消し判定を含めること（6.4.3 の既返信チェックと同じタイミングで行う）。

### 5.2 app.db（自前 SQLite）

パス: `~/Library/Application Support/net.votepurchase.momreply/app.db`

```sql
CREATE TABLE IF NOT EXISTS processed_messages (
  chat_rowid       INTEGER PRIMARY KEY,   -- chat.db の message.ROWID
  chat_guid        TEXT    NOT NULL,
  received_at      INTEGER NOT NULL,      -- unix秒
  body             TEXT,
  status           TEXT    NOT NULL,      -- pending|generating|awaiting_review|sent|skipped|failed|dry_run
  skip_reason      TEXT,                  -- stale|already_replied|rate_limited|kill_switch|excluded|empty_body
  draft            TEXT,
  final_text       TEXT,
  sent_at          INTEGER,
  sent_rowid       INTEGER,               -- 送信確認できた chat.db の ROWID
  provider         TEXT,
  model            TEXT,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS generation_log (
  id                INTEGER PRIMARY KEY AUTOINCREMENT,
  chat_rowid        INTEGER,
  kind              TEXT NOT NULL,        -- initial|regenerate|profile_update
  provider          TEXT,
  model             TEXT,
  input_tokens      INTEGER,
  output_tokens     INTEGER,
  latency_ms        INTEGER,
  user_instruction  TEXT,                 -- 再生成時のユーザー指示
  output            TEXT,
  error             TEXT,
  created_at        INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS profile_revisions (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  content       TEXT    NOT NULL,         -- 適用後のプロファイル全文
  source        TEXT    NOT NULL,         -- manual|ai
  change_note   TEXT,                     -- AI 更新時の差分説明
  source_rowids TEXT,                     -- JSON配列。根拠となった chat.db の ROWID
  applied       INTEGER NOT NULL DEFAULT 0,
  created_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS fewshot_pairs (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  incoming       TEXT    NOT NULL,        -- 母の発言
  reply          TEXT    NOT NULL,        -- 自分の返信
  source_rowid   INTEGER NOT NULL,
  built_at       INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS kv (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
-- 使用キー:
--   last_seen_rowid           : 最後に処理した chat.db の ROWID
--   last_poll_at              : 最後にポーリングした unix秒
--   dry_run_started_at        : ドライラン開始日時
--   auto_send_enabled         : "true" | "false"（キルスイッチ）
--   consecutive_auto_count    : 連続自動返信回数（6.4.5.1）
--   session_started_at        : 現在の会話セッション開始 unix秒
--   session_length_preset     : セッション単位の長さ上書き（空なら既定値）
--   budget_notified_month     : ソフト上限の通知済み月（"2026-08"）
```

### 5.3 母プロファイル

パス: `~/Library/Application Support/net.votepurchase.momreply/profile.md`

人間が読める Markdown で保存する。app.db にはリビジョン履歴のみ持つ。

```markdown
# 母プロファイル

## 基本
- 呼び方: （自分が母をどう呼ぶか）
- 自分の呼ばれ方:
- 居住地:

## 家族・人間関係
- （名前と続柄。会話に出てくる人）

## 健康・通院
- （持病、通院先、服薬）

## 予定・イベント
- （旅行、法事、誕生日など。日付付き）

## 会話のクセ
- （よく使う言い回し、話題の傾向）

## 触れないほうがいいこと
- （地雷）
```

---

## 6. 機能仕様

### 6.1 メッセージ監視（watcher.rs）

#### トリガー

以下の 3 つを併用する。

1. **定期ポーリング**: 5 秒間隔で `last_seen_rowid` より新しい行を取得
2. **FSEvents 監視**: `notify` クレートで `chat.db-wal` の変更を監視し、即座にポーリングを起動（デバウンス 500ms）
3. **スリープ復帰検知**: `NSWorkspace.didWakeNotification` を購読。復帰時にポーリングを起動

#### 初回起動時のバックログ保護（必須）

`last_seen_rowid` が未設定の状態で通常のポーリングを走らせると、**過去の全メッセージが「新着」として処理対象になる**。全自動モードなら数百通に一斉返信する重大事故になる。

```
kv.last_seen_rowid が存在しない
  ↓
母との会話の現在の最大 ROWID を取得
  ↓
last_seen_rowid にその値をセットして終了
  ↓
既存メッセージは一切処理しない（processed_messages にも記録しない）
```

同じ保護を以下のケースにも適用する。

- app.db が削除・破損して再作成された場合
- `target.handles` が変更された場合（新しいハンドルの過去分は処理しない）
- ドライランから本番へ切り替えた場合（切替時点より前のメッセージは対象外）

**開発中の注意**: app.db を消して再起動するたびにこの初期化が走る。テストで過去メッセージを処理させたい場合は、`last_seen_rowid` を手動で書き換えること。コードで初期化をスキップできるようにしてはいけない。

#### スリープ復帰時の挙動（重要）

ノート PC のためスリープが頻発する。復帰時に溜まったメッセージを一斉処理すると、深夜のメッセージに朝まとめて自動返信する事故が起きる。

```
復帰検知 or ポーリング時の時刻ギャップ検出（前回 last_poll_at から 10分以上経過）
  ↓
未処理メッセージを全件取得
  ↓
最新の1件のみを生成対象とする
残りは status = 'skipped', skip_reason = 'stale' として記録
  ↓
最新1件も stale guard（6.4.2）の判定にかける
```

### 6.2 返信生成（pipeline）

```
新着検知
  ↓
ガード判定（6.4）── NG ─→ skipped として記録 / 必要なら通知
  ↓ OK
コンテキスト構築
  - 母プロファイル（profile.md 全文）
  - few-shot ペア（fewshot_pairs から最大 40 組）
  - 直近の会話履歴（母との直近 20 メッセージ、自分の返信含む）
  - 現在日時・曜日
  ↓
LLM 呼び出し（プライマリプロバイダ）
  ↓ 失敗
  リトライ（指数バックオフ、最大3回）
  ↓ なお失敗
  フォールバックプロバイダへ（設定されていれば）
  ↓ なお失敗
  status = 'failed' で記録し、通知して終了（何も送らない）
  ↓ 成功
後処理（6.2.1）
  ↓
全自動モード ON → 送信（6.3）
全自動モード OFF → status = 'awaiting_review'、通知してキューへ
```

#### 6.2.1 生成結果の後処理

LLM の出力に対して以下を必ず適用する。

1. 前後の空白・改行をトリム
2. 出力を囲むクォート（`"` `「」`）が全体を囲んでいる場合は除去
3. `返信:` `以下のように返信します:` などの前置きが行頭にある場合は除去
4. コードブロック記法（``` ）が含まれていたら中身を取り出す
5. `hard_max_length`（6.9 参照）を超えていたら **送信せず** `awaiting_review` に倒す（暴走検知）
6. 空文字なら `failed` として扱う

### 6.3 送信（sender.rs）

#### AppleScript

エスケープ事故を防ぐため、本文は必ず `argv` で渡す。文字列連結でスクリプトを組み立てないこと。

#### 送信先ハンドルの決定ルール（必須）

`target.handles` は配列である。母が電話番号と Apple ID の両方を持つ場合、chat.db 上に別々の `chat` 行が存在し、会話が2本に分かれている。

**ルール: 受信したメッセージと同じ `chat_identifier` に返す。**

```
processed_messages.chat_guid に受信元の chat_identifier を保存
  ↓
送信時はその値を osascript の第1引数に渡す
  ↓
設定の handles 配列の順序や先頭要素は送信先の決定に使わない
```

これを守らないと、SMS のスレッドで受けた話に iMessage のスレッドで返すことになり、母の画面では会話が2本に割れて見える。

`send.applescript`:

```applescript
on run argv
	set targetBuddy to item 1 of argv
	set targetMessage to item 2 of argv
	tell application "Messages"
		set targetService to 1st account whose service type = iMessage
		set theBuddy to participant targetBuddy of targetService
		send targetMessage to theBuddy
	end tell
end run
```

呼び出し:

```rust
Command::new("osascript")
    .arg(script_path)
    .arg(&handle)      // 母の電話番号 or Apple ID
    .arg(&text)
    .output()?;
```

#### 送信成否の検証（必須）

AppleScript は送信に失敗してもエラーを返さないことがある。`osascript` の終了コードだけを信じてはいけない。

```
osascript 実行
  ↓
1秒間隔で chat.db をポーリング（最大 30 秒）
  ↓
条件: chat_identifier が母 かつ is_from_me = 1 かつ ROWID > 送信前の最大ROWID
      かつ 本文が送信テキストと一致（正規化して比較）
  ↓ 見つかった
sent_rowid を記録、status = 'sent'
  ↓ 30秒経っても見つからない
status = 'failed'、通知「送信に失敗した可能性があります」
※ 自動リトライはしない（二重送信のリスクのため）
```

初回起動時、Messages.app が起動していない場合は AppleScript が起動を試みる。起動待ちのため初回のみタイムアウトを 60 秒にする。

### 6.4 セーフティガード（guards.rs）

**すべて必須。1つでも欠けると事故が起きる。**

#### 6.4.1 Allowlist

設定された母のハンドル（複数可: 電話番号・Apple ID）以外は、SQL の WHERE 句の時点で除外する。取得してからフィルタするのではなく、**そもそも読み込まない**。

#### 6.4.2 Stale guard

```
now - received_at > stale_threshold_minutes（初期値 15）
  → 自動送信しない。status = 'awaiting_review' にして手動キューへ
```

#### 6.4.3 既返信チェック（最重要）

あなたが iPhone / iPad で既に手で返信している可能性がある。これが無いと二重返信する。

```sql
SELECT COUNT(*)
FROM message m
JOIN chat_message_join cmj ON cmj.message_id = m.ROWID
JOIN chat c               ON c.ROWID = cmj.chat_id
WHERE c.chat_identifier IN (:handles)
  AND m.is_from_me = 1
  AND m.ROWID > :target_rowid;
```

1 以上なら `skip_reason = 'already_replied'` でスキップ。

**このチェックは生成の直前と送信の直前の 2 回行う**（生成に数秒かかる間に手動返信される可能性があるため）。

#### 6.4.4 ループ防止

- `is_from_me = 1` の行は絶対にトリガーにしない
- 自分が送信した直後の 60 秒間は新規トリガーを受け付けない（クールダウン）

#### 6.4.5 レートリミット

| 項目 | 設定キー | 初期値 |
|---|---|---|
| 1時間あたりの自動送信上限 | `max_per_hour` | 6 通 |
| 1日あたりの自動送信上限 | `max_per_day` | 30 通 |
| **連続自動返信の上限** | `max_consecutive_auto_replies` | **5 回** |

いずれかの上限に達したら自動送信を停止し、以降は `awaiting_review` に倒して通知する。

#### 6.4.5.1 連続自動返信の上限（会話セッション）

全自動モードでやり取りが延々続くと、API 課金が青天井になるうえ、人間が一度も内容を見ないまま長い会話が成立してしまう。これを「会話セッション」という単位で打ち切る。

**セッションの定義**

```
最後のメッセージ（送受信どちらでも）から session_gap_minutes（初期値 180）以内に
続いている一連のやり取りを 1 セッションとする。
```

**カウント**

`kv.consecutive_auto_count` に整数で保持する。

| イベント | 挙動 |
|---|---|
| 自動送信に成功 | +1 |
| ユーザーが UI から手動送信した | **0 にリセット** |
| ユーザーが返信案を編集した | **0 にリセット** |
| ユーザーが `awaiting_review` を明示的にスキップした | **0 にリセット** |
| 前回のやり取りから `session_gap_minutes` 以上経過 | **0 にリセット** |
| 日付が変わった | **0 にリセット** |
| UI の「リセット」ボタン | 0 にリセット |

**上限到達時**

```
consecutive_auto_count >= max_consecutive_auto_replies
  ↓
以降のメッセージは生成はするが自動送信せず status = 'awaiting_review'
  ↓
通知: 「連続5回自動返信しました。ここから先は確認モードです」
  ↓
ポップオーバーに「自動を再開」ボタンを表示（押すとカウンタをリセット）
```

**重要**: 上限に達しても LLM 呼び出し自体は行われるため、課金は発生し続ける。生成すら止めたい場合のために `pause_generation_on_limit`（初期値 `false`）を用意し、`true` のときは生成もスキップして通知のみ出す。

#### 6.4.5.2 コスト上限（バックストップ）

連続回数の上限だけでは、時間をおいた散発的なやり取りが積み重なるケースを止められない。月次の金額上限を最終防衛線として置く。

```
generation_log の input_tokens / output_tokens を集計
  × 設定に持つモデル別単価（settings に記載。ハードコードしない）
  = 当月の推定コスト
```

| 設定 | 初期値 | 挙動 |
|---|---|---|
| `monthly_soft_limit_usd` | 3.0 | 到達したら通知のみ（動作は継続） |
| `monthly_hard_limit_usd` | 10.0 | 到達したら自動送信と生成を停止。手動送信のみ可能 |

推定コストはポップオーバーの設定画面に常時表示する。単価が設定に無いモデルはコスト 0 として扱い、「単価未設定」と表示する。

#### 6.4.6 キルスイッチ

メニューバーのメニューから 1 クリックで全自動モードを OFF にできる。状態は `kv.auto_send_enabled` に永続化。OFF 中はトレイアイコンの見た目を変える。

#### 6.4.7 ドライラン

`dry_run = true` の間は、生成まで行って送信せず `status = 'dry_run'` で記録する。UI 上では「送信するはずだった内容」として表示する。

**初期設定は `dry_run = true`。** 実運用前に必ず一定期間の検証を挟むこと。

#### 6.4.8 エスカレーション（初期値 OFF）

特定条件に合致したメッセージを自動送信せず `awaiting_review` に倒す機構。実装はするが、設定の初期値は空とする。

```toml
[escalation]
enabled = false
keywords = ["病院", "入院", "手術", "お金", "振り込", "亡くな", "急い"]
escalate_on_question = false   # 「?」「？」を含む場合
```

### 6.5 全自動モード

| 項目 | 仕様 |
|---|---|
| 送信タイミング | 生成完了後、即座に送信（`auto_send_delay_seconds` 初期値 0） |
| 実質的な遅延 | LLM の往復に 3〜10 秒かかるため、受信から送信まではその程度になる |
| 送信後 | 必ず通知を出し、送信した本文を表示する |
| 通知タップ | ポップオーバーを開き、該当メッセージの詳細を表示 |

`auto_send_delay_seconds` を 0 より大きくした場合は、遅延中に通知を出し、タップで編集・キャンセルできるようにする。

### 6.6 手動確認 UI（メニューバーポップオーバー）

トレイアイコンクリックで幅 380px 程度のポップオーバーを開く。

#### レイアウト

```
┌────────────────────────────────────┐
│ 母  ●自動ON            [設定] [×]  │
├────────────────────────────────────┤
│ 直近の会話（スクロール）             │
│   母: 明日そっち行くけど大丈夫？      │
│   14:32                             │
├────────────────────────────────────┤
│ 返信案                              │
│ ┌────────────────────────────────┐ │
│ │ 大丈夫だよ、何時ごろ来る？       │ │  ← 直接編集可
│ │                                │ │
│ └────────────────────────────────┘ │
│ 長さ: [短め] [ふつう] [長め] [激長] │  ← 押すと即再生成
│ ┌────────────────────────────────┐ │
│ │ AIへの指示（任意）              │ │
│ └────────────────────────────────┘ │
│        [再生成]        [送信]       │
├────────────────────────────────────┤
│ 自動返信 3/5 回  今月 $0.42  [リセット]│
├────────────────────────────────────┤
│ 履歴 ▾                              │
│  14:02 送信済「うん、わかった」      │
│  昨 21:15 送信済「ありがとう」       │
└────────────────────────────────────┘
```

#### 挙動

- 返信案のテキストエリアは**フォーカスされた状態で開く**（直接編集が主動線のため）
- `⌘Enter` で送信
- 「AIへの指示」が空のまま再生成した場合は同じコンテキストで再生成、入力があればそれを追加指示としてプロンプトに含める
- 未処理（`awaiting_review`）が複数ある場合はバッジ数を表示し、新しい順に切り替えられる
- 履歴セクションでは過去 50 件の送信/スキップ結果を確認できる

#### トレイアイコンの状態

| 状態 | 表示 |
|---|---|
| 通常（自動ON） | 通常アイコン |
| 自動OFF | グレーアウト |
| ドライラン中 | 点線バッジ |
| 未確認あり | 数字バッジ |
| エラーあり | 赤バッジ |

### 6.7 母プロファイルの AI 自動更新

#### 実行タイミング

- 母とのやり取りが累計 20 通進むごと
- または最終更新から 7 日経過し、かつ新しいやり取りがある場合
- UI から手動実行も可能

#### 処理

```
直近の未反映会話（最大 100 メッセージ）+ 現在の profile.md
  ↓
LLM に「追記・修正すべき事実」を JSON で抽出させる
  ↓
差分を profile_revisions に applied = 0 で保存
  ↓
通知「プロファイルの更新候補が3件あります」
  ↓
UI で差分を確認 → 承認 or 却下（項目ごと）
  ↓
承認分を profile.md に反映、applied = 1
```

**自動反映はしない。** 誤情報がプロファイルに混入すると以後すべての生成が汚染されるため、必ず人間が承認する。

#### 抽出プロンプトの出力形式

```json
{
  "updates": [
    {
      "section": "健康・通院",
      "operation": "add",
      "content": "膝の治療で〇〇整形外科に通院中（2026年7月〜）",
      "evidence_rowid": 123456,
      "confidence": "high"
    },
    {
      "section": "予定・イベント",
      "operation": "update",
      "old_content": "8月に温泉旅行の予定",
      "content": "8月の温泉旅行は9月に延期",
      "evidence_rowid": 123460,
      "confidence": "medium"
    }
  ]
}
```

`confidence: "low"` の項目はデフォルトで未チェック状態で提示する。

### 6.8 few-shot 抽出（fewshot.rs）

#### 抽出ロジック

```
母との会話から (母の発言, 直後の自分の返信) のペアを構築
  ↓
フィルタ:
  - 自分の返信が 2文字未満 → 除外（「w」など）
  - 自分の返信が 200文字超 → 除外
  - 母の発言が空（添付のみ）→ 除外
  - タップバック・システムメッセージ → 除外
  - 同一の返信文が既に3回出現 → 4回目以降は除外（「うん」の偏り防止）
  ↓
直近 200 ペアから、以下を混ぜて最大 40 ペアを選定:
  - 直近 20 ペア（最新の文体を反映）
  - 残りの 180 ペアからランダムに 20 ペア（多様性確保）
  ↓
fewshot_pairs に保存
```

#### 更新タイミング

アプリ起動時、および 1 日 1 回。

#### 初回構築

初回起動時は過去全体から構築する。会話量が多い場合は直近 1 年分に限定してよい。

### 6.9 返信の長さ制御

返信の長さは**ハードコードせず、プリセットで切り替えられる**ようにする。「わざと長文を書かせたい」「今回だけ長めに」といった用途に対応するため。

#### 6.9.1 プリセット

| プリセット | 目標文字数 | `hard_max_length` | 想定用途 |
|---|---|---|---|
| `mirror` | 母のメッセージ長 × 0.8〜1.5 | 300 | **デフォルト。**もっとも自然 |
| `short` | 10〜40 | 150 | 相槌中心 |
| `normal` | 30〜100 | 300 | |
| `long` | 200〜400 | 800 | 近況をたっぷり書く |
| `very_long` | 600〜1200 | 2000 | 意図的に読ませる長文 |
| `custom` | `min_chars` 〜 `max_chars` | `hard_max_length` | 手動指定 |

`hard_max_length` は生成の目標ではなく**暴走検知の閾値**である。これを超えた出力は送信せず `awaiting_review` に倒す（6.2.1-5）。目標上限のおよそ 2 倍を目安に設定する。

#### 6.9.2 適用レイヤー

長さ設定は 3 段階で解決する。後のものが優先される。

1. `reply_length.default_preset`（グローバル設定）
2. セッション単位の一時上書き（ポップオーバーのトグルで変更、セッション終了でリセット）
3. ワンオフの上書き（その 1 回の生成にだけ適用）

#### 6.9.3 プロンプトへの反映

システムプロンプトの `{length_instruction}` に、プリセットに応じた文言を差し込む。

```
mirror      → 母のメッセージと同じくらいの長さで返す。相手が一言なら一言で返す。
short       → 10〜40文字程度。1文で簡潔に。
normal      → 30〜100文字程度。1〜2文。
long        → 200〜400文字程度。近況や感想を具体的に添えて、3〜5文程度で書く。
             ただし文体は文例のまま崩さないこと。
very_long   → 600〜1200文字程度。近況、感想、質問、思い出話などを織り交ぜて
             たっぷり書く。改行を使って読みやすくする。
             ただし文体・語尾・絵文字の使い方は文例のまま崩さないこと。
             丁寧語やビジネス文体に寄せてはいけない。
```

#### 6.9.4 few-shot との衝突（実装上の注意）

**`long` / `very_long` は few-shot に負けやすい。** few-shot に短い返信ばかり並んでいると、システムプロンプトで長文を指示しても短い出力に引っ張られる。対策を以下の順で試すこと。

1. 長文プリセット時は、`fewshot_pairs` から**自分の返信が長いペアを優先して選ぶ**（返信の文字数で降順ソートし、上位から 20 組 + 直近 10 組）
2. それでも短い場合は few-shot の件数を 40 → 15 に減らし、システムプロンプトの指示を相対的に強める
3. 最後の user メッセージの直後に長さ指示を再掲する（末尾の指示のほうが効きやすい）

長文ペアが 5 組未満しか存在しない場合は、UI に「長文の文例が少ないため、文体の再現度が落ちる可能性があります」と注記を出す。

#### 6.9.5 UI

ポップオーバーの返信案の下に長さトグルを置き、押すと**その場で再生成**する。

```
長さ:  [短め]  [ふつう]  [長め]  [激長]
```

- 現在適用中のプリセットをハイライトする
- 押した瞬間に再生成が走る（別途「再生成」を押させない）
- 選択はそのセッションの間だけ保持される

#### 6.9.6 コストへの影響（警告）

`very_long` は 1 回あたりの出力トークンが `mirror` の 10〜30 倍になる。さらに長い返信は相手も返信しやすくなるため往復数自体が増え、次回以降の会話履歴も膨らむ。**全自動モード × 長文プリセットの組み合わせは、コストが二乗的に増える**。

そのため以下の制約を入れる。

- 全自動モードで `long` 以上のプリセットが選択されている場合、初回の自動送信前に一度だけ確認ダイアログを出す
- `max_consecutive_auto_replies` は、`long` 以上のときは設定値と 3 の小さいほうを使う

---

## 7. LLM プロバイダ抽象化

### 7.1 trait 定義

```rust
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    fn id(&self) -> &'static str;
    async fn complete(&self, req: CompletionRequest)
        -> Result<CompletionResponse, LlmError>;
}

pub struct CompletionRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<ChatMessage>,   // role: user | assistant
    pub max_tokens: u32,
    pub temperature: f32,
}

pub struct CompletionResponse {
    pub text: String,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub latency_ms: u64,
}

pub enum LlmError {
    Auth,           // APIキー不正 → 通知してリトライしない
    RateLimit,      // → バックオフしてリトライ
    Network,        // → リトライ
    Server,         // → リトライ
    InvalidOutput,  // → リトライしない
}
```

### 7.2 各プロバイダ

| プロバイダ | エンドポイント | 認証ヘッダ | 備考 |
|---|---|---|---|
| Anthropic | `POST https://api.anthropic.com/v1/messages` | `x-api-key` + `anthropic-version: 2023-06-01` | system は独立フィールド |
| OpenAI | `POST https://api.openai.com/v1/chat/completions` | `Authorization: Bearer` | system は messages の先頭 |
| Gemini | `POST https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent` | `x-goog-api-key` | `systemInstruction` フィールド、role は user/model |
| Apple Intelligence | Swift サイドカー経由（HTTP なし） | 不要 | オンデバイス。7.3 参照 |

**モデル名は設定でユーザーが自由に指定できるようにする**（ハードコードしない）。デフォルト値は設定ファイルに置き、実装時点で利用可能な最新モデルを記載する。

### 7.3 Apple Intelligence プロバイダ（オンデバイス）

#### 7.3.1 位置づけ

**返信生成の主役には使わない。** 理由はコンテキストウィンドウで、Apple のドキュメントはオンデバイスモデルを 1 セッションあたり 4,096 トークンとしている。本仕様の生成コンテキスト（プロファイル + few-shot 40組 + 直近20メッセージ）は日本語だとこれに収まらず、few-shot を削ると文体再現度が落ちて本アプリの価値が失われる。

代わりに**精度要求が低く呼び出し頻度が高いサブタスク**を担当させる。無料・オフライン・会話が端末外に出ないという利点がそのまま効く領域である。

#### 7.3.2 タスク別ルーティング

| タスク | 既定プロバイダ | 理由 |
|---|---|---|
| 返信生成 | `llm.primary`（Claude / Gemini） | few-shot をフルに使う必要がある |
| 再生成 | `llm.primary` | 同上 |
| エスカレーション判定・分類 | `apple` | 短文分類は小型モデルの得意分野。呼び出し頻度が最も高い |
| 古い会話履歴の要約圧縮 | `apple` | 精度要求が低い |
| 長さプリセットの `mirror` 判定補助 | `apple` | 文字数計算の補助のみ |
| プロファイル更新候補の抽出 | `llm.primary` | 誤情報が混入すると以後の生成が汚染されるため精度優先 |

ルーティングは設定で個別に上書きできること。`apple` が利用不可の環境では自動的に `llm.primary` にフォールバックする。

#### 7.3.3 Swift サイドカー

Foundation Models は Swift API であり Rust から直接呼べない。小さな CLI を作って Tauri のサイドカーとして同梱する。

```
src-sidecar/
└── momreply-fm/        # Swift Package（バイナリ名: momreply-fm）
    └── Sources/main.swift
```

**インターフェース**（プロセス起動オーバーヘッドを避けるため、1リクエスト1プロセスではなく標準入出力の行区切り JSON で常駐させる）

```
stdin  → {"id":"1","system":"...","messages":[...],"max_tokens":200}
stdout → {"id":"1","ok":true,"text":"...","input_tokens":312,"output_tokens":18}
stdout → {"id":"1","ok":false,"error":"context_window_exceeded"}
```

**起動時のヘルスチェック**

サイドカー起動直後に `{"op":"availability"}` を投げ、以下を取得して app.db に保存する。

- `SystemLanguageModel.availability`（`.available` / `.unavailable(reason)`）
- `SystemLanguageModel.contextSize`（**ハードコードせず実行時に読む**。将来のハードウェア・OS で変わる）

`.unavailable` の場合は `apple` を選択肢から外し、UI に理由を表示する。想定される理由: Apple Intelligence 未有効、非対応ハードウェア、リージョン制限、モデルのダウンロード中。

#### 7.3.4 実装上の注意

1. **プロンプト長は必ず事前に測る**。`tokenCount(for:)` で見積もり、`contextSize` の 80% を超えるなら `llm.primary` にフォールバックする。実行してから `contextWindowExceeded` を食うと無駄なレイテンシになる
2. **Foundation Models にはセーフティガードレールがある**。家族の雑談で発火することは稀だが、拒否された場合は `LlmError::InvalidOutput` として扱い、`llm.primary` にフォールバックする
3. **生成速度は端末負荷に依存する**。他の Neural Engine ワークロードと競合すると遅くなるため、タイムアウトは 30 秒と長めに取る
4. **課金カウントの対象外**。`generation_log` には記録するが、`budget` の集計からは除外する（単価 0）

#### 7.3.5 macOS 27 での再評価（重要）

WWDC 2026 で `PrivateCloudComputeLanguageModel` が発表された。Apple Intelligence を支えるサーバーモデルが同じセッション API 経由で開放され、32,000 トークンのコンテキストと reasoning が使える。これは **OS 27 が必要**で、本仕様書の作成時点ではベータである。

macOS 27 が一般提供されたら以下を再評価すること。

- 32K あれば few-shot 40組 + プロファイル + 直近履歴が余裕で入る。**返信生成そのものを Apple 側に移せる可能性がある**
- macOS 27 では Foundation Models の Python SDK と `fm` CLI が提供されるため、自作の Swift サイドカーが不要になる可能性がある
- Anthropic / Google が同プロトコル準拠の Swift パッケージを出しており、本仕様の `LlmProvider` trait を Apple の `LanguageModel` プロトコルに置き換えて統一できる可能性がある

移行判断の基準: 32K モデルで生成した返信の文体再現度が、現行の Claude / Gemini と同等かどうか。ドライランで 1 週間比較して決めること。

### 7.4 設定

```toml
[llm]
primary  = "anthropic"
fallback = "gemini"        # 空文字ならフォールバックなし

[llm.anthropic]
model = "..."
max_tokens = 500
temperature = 0.8

[llm.gemini]
model = "..."

[llm.openai]
model = "..."

[llm.apple]
enabled = true
# contextSize はハードコードせず実行時に SystemLanguageModel から読む。
# 下記はプロンプトを組む際の安全マージン（読み取った値に対する割合）
context_safety_ratio = 0.8
timeout_seconds = 30

# タスク別ルーティング（7.3.2）。"apple" が利用不可なら自動で primary にフォールバック
[llm.routing]
reply_generation   = "primary"
regeneration       = "primary"
classification     = "apple"
history_summary    = "apple"
profile_extraction = "primary"
```

### 7.5 API キーの管理

**API キーは UI から入力する。** 設定ファイルの手編集や環境変数は使わない（`apple` プロバイダはキー不要）。

#### 7.5.1 保存先

macOS Keychain。`config.toml` にも `app.db` にもログにも書かない。

```rust
let entry = keyring::Entry::new(
    "net.votepurchase.momreply",
    "anthropic_api_key",      // gemini_api_key / openai_api_key
)?;
entry.set_password(&key)?;
```

#### 7.5.2 設計原則（重要）

**キーはフロントエンドに一度も返さない。** React 側が保持するのは「設定済みかどうか」の状態だけ。

```
入力時:  React → Tauri command(set_api_key) → Keychain
                 ↑ ここで React 側の state は即座にクリアする

読取時:  React ← Tauri command(get_key_status) ← Keychain
                 返るのは { status, masked } のみ。キー本体は返さない
```

`masked` は末尾4文字のみ（`sk-...a3f9`）。前方一致で漏れると意味がないため、**先頭は絶対に返さない**。

#### 7.5.3 Tauri コマンド

```rust
#[tauri::command]
async fn set_api_key(provider: String, key: String) -> Result<KeyStatus, String>;
// Keychain に保存 → 疎通テスト（7.5.5）→ 結果を返す

#[tauri::command]
fn get_key_status(provider: String) -> KeyStatus;
// { configured: bool, verified: bool, masked: Option<String>, last_verified_at: Option<i64> }

#[tauri::command]
fn delete_api_key(provider: String) -> Result<(), String>;

#[tauri::command]
async fn verify_api_key(provider: String) -> Result<KeyStatus, String>;
// 保存済みキーで再テスト。キーを受け取らない
```

キー本体を戻り値に含むコマンドを作らないこと。デバッグ目的でも作らない。

#### 7.5.4 UI

設定画面（ポップオーバーの歯車から開く）に、プロバイダごとに1行。

```
┌──────────────────────────────────────────┐
│ APIキー                                   │
├──────────────────────────────────────────┤
│ Anthropic    ●検証済み  sk-...a3f9        │
│              [再検証] [削除]              │
├──────────────────────────────────────────┤
│ Gemini       ○未設定                      │
│              [••••••••••••••••••] [保存]  │
├──────────────────────────────────────────┤
│ OpenAI       ⚠検証失敗  sk-...7b21        │
│              401 Unauthorized             │
│              [••••••••••••••••••] [保存]  │
├──────────────────────────────────────────┤
│ Apple Intelligence  ●利用可能（キー不要）  │
└──────────────────────────────────────────┘
```

実装上の要件:

- `<input type="password">`、`autocomplete="off"`、`spellcheck="false"`
- ペースト可（手打ちさせない）。前後の空白と改行は自動でトリムする
- 保存成功後、input の value を即座に空にする
- 状態は3種: `未設定` / `検証済み` / `検証失敗`（失敗時はエラー内容を表示）
- **キーが1つも設定されていない状態では自動送信を有効にできない**（トグルを無効化し理由を表示）
- `llm.primary` に設定されたプロバイダのキーが未設定なら、設定画面に警告を出す

#### 7.5.5 疎通テスト

保存時に必ず実行する。「保存できた」と「使える」は別。

各プロバイダで**最も安価な呼び出し**を1回行う。

| プロバイダ | テスト内容 |
|---|---|
| Anthropic | `max_tokens: 1` で `"hi"` を1回送る |
| OpenAI | `max_tokens: 1` で1回送る |
| Gemini | `maxOutputTokens: 1` で1回送る |

結果の扱い:

| 応答 | 表示 |
|---|---|
| 200 | `検証済み`。`last_verified_at` を記録 |
| 401 / 403 | `検証失敗: キーが正しくありません` |
| 429 | `保存済み（レート制限のため未検証）`。キーは保持する |
| ネットワークエラー | `保存済み（未検証）`。キーは保持する |

401/403 の場合も**キーは Keychain に保存する**（打ち間違いを直したいときに再入力させないため）。ただし `verified: false` として記録し、自動送信の前提条件から外す。

#### 7.5.6 ログとエラーメッセージ

- キーの一部でもログに出さない。`tracing` のフィールドに乗せない
- HTTP エラー時、リクエストヘッダをそのままログに書かないこと（`x-api-key` が乗る）
- Tauri の IPC はローカルだが、`console.log` にキーを出さない

#### 7.5.7 開発中の注意

Keychain のアイテムはコード署名に紐づく。**dev ビルドは再ビルドのたびに署名が変わるため、毎回 Keychain のアクセス許可ダイアログが出るか、読み取りに失敗する。**

対処:

- `tauri.conf.json` で ad-hoc 署名の identity を固定する
- またはダイアログで「常に許可」を選ぶ（署名が変わると再度出る）
- 開発中に限り、環境変数 `MOMREPLY_DEV_API_KEY_*` からの読み取りをフォールバックとして許可してよい。ただし **`#[cfg(debug_assertions)]` で囲み、リリースビルドに絶対に含めないこと**

---

## 8. プロンプト設計

### 8.1 返信生成のシステムプロンプト

```
あなたは「{user_name}」本人として、母親からの iMessage に返信を書きます。
アシスタントではなく、本人になりきってください。

# 絶対のルール
- 返信の本文のみを出力する。前置き・説明・引用符・コードブロックは一切つけない
- 「〜という返信はいかがでしょうか」のようなメタ発言は禁止
- 以下の文例と同じ文体・語尾・絵文字の使い方を厳密に真似る
- 確信のない事実（自分の予定、金額、日程）を断定しない。曖昧なら「確認してみる」と返す
- 存在しない出来事をでっち上げない

# 返信の長さ
{length_instruction}
※ 長さの指示は文体より優先されるが、文体そのものを変えてはならない。
　 長く書く場合も、文例と同じくだけた話し言葉のまま書くこと。

# 母について
{profile_md}

# 現在
{current_datetime}（{weekday}）

# 直近の会話
{recent_conversation}
```

### 8.2 few-shot の渡し方

`messages` 配列に user / assistant のペアとして展開する。system プロンプト内にテキストとして埋め込むより、実際の会話形式のほうが文体の模倣精度が高い。

```
messages = [
  { role: "user",      content: "<母> ごはん食べた？" },
  { role: "assistant", content: "食べたよー" },
  { role: "user",      content: "<母> 明日雨だって" },
  { role: "assistant", content: "まじか、傘持ってく" },
  ...（40ペア）...
  { role: "user",      content: "<母> {今回のメッセージ}" }
]
```

### 8.3 再生成時

ユーザーの追加指示を最後の user メッセージの後に付加する。

```
{ role: "assistant", content: "{前回の生成結果}" },
{ role: "user",      content: "この返信を次の指示で書き直して: {user_instruction}\n本文のみ出力すること。" }
```

---

## 9. 設定項目一覧

```toml
[target]
handles = ["+81xxxxxxxxxx", "mother@icloud.com"]
display_name = "母"

[auto_send]
enabled = true
dry_run = true                      # 初期値。検証後に false へ
delay_seconds = 0
stale_threshold_minutes = 15
max_per_hour = 6
max_per_day = 30
cooldown_after_send_seconds = 60

# 連続自動返信の上限（6.4.5.1）
max_consecutive_auto_replies = 5
session_gap_minutes = 180           # これ以上空いたらセッション終了・カウンタリセット
pause_generation_on_limit = false   # true なら上限到達後は生成もしない

[reply_length]
default_preset = "mirror"           # mirror|short|normal|long|very_long|custom
# custom のときのみ使用
min_chars = 30
max_chars = 100
# 暴走検知の閾値。プリセットごとの既定値を上書きしたいときに指定
hard_max_length = 300

[budget]
monthly_soft_limit_usd = 3.0        # 通知のみ
monthly_hard_limit_usd = 10.0       # 自動送信・生成を停止
# モデル別単価（USD / 100万トークン）。実装時点の公式価格を記載すること
[budget.pricing]
# "provider:model" = { input = 0.0, output = 0.0 }

[escalation]
enabled = false
keywords = []
escalate_on_question = false

[watcher]
poll_interval_seconds = 5
wake_gap_threshold_minutes = 10
prevent_sleep_on_ac = true          # 電源接続時のみ caffeinate 相当

[context]
recent_messages = 20
fewshot_pairs = 40

[profile]
auto_update_every_n_messages = 20
auto_update_min_days = 7

[llm]
# 7.4 参照（タスク別ルーティング・Apple Intelligence 設定を含む）

[log]
level = "info"
retain_days = 90
```

---

## 10. 権限とセットアップ

### 10.1 必要な権限

| 権限 | 用途 | 付与場所 |
|---|---|---|
| フルディスクアクセス | chat.db の読み取り | システム設定 → プライバシーとセキュリティ → フルディスクアクセス |
| オートメーション（Messages） | AppleScript による送信 | 初回送信時にダイアログ。システム設定 → オートメーション |
| 通知 | 送信結果の通知 | 初回起動時にダイアログ |

`Info.plist` に `NSAppleEventsUsageDescription` を記載すること。記載がないとオートメーション権限のダイアログが出ずに失敗する。

### 10.2 オンボーディング画面

初回起動時に以下を順に案内する。

1. フルディスクアクセスの付与（設定画面を直接開くボタン。付与後はアプリ再起動が必要な旨を明示）
2. chat.db の読み取りテスト（成功したら会話一覧を表示）
3. 母のハンドル選択（chat.db から会話相手一覧を出して選ばせる。手打ちさせない）
4. API キーの入力（7.5 の UI。1 プロバイダ以上。ペースト前提、`type="password"`）
5. 疎通テスト（保存時に自動実行。成功したら実際に 1 回生成してみせる）
6. few-shot の初回構築（進捗表示）
7. プロファイルの初期生成（過去の会話から AI に叩き台を作らせ、ユーザーが編集）
8. ログイン時に起動する設定

---

## 11. エラーハンドリング

| 事象 | 挙動 |
|---|---|
| chat.db が開けない | トレイに赤バッジ。「フルディスクアクセスを確認してください」＋設定を開くボタン |
| attributedBody のデコード失敗 | `m.text` にフォールバック。それも空なら該当メッセージをスキップしてログに残す |
| API キー未設定 / 401 | 自動送信を停止。通知。リトライしない |
| API レート制限 | 指数バックオフで最大3回リトライ。その後フォールバックプロバイダ |
| 生成が空 / 300文字超 | 送信せず `awaiting_review` へ |
| osascript 失敗 | `failed` として記録、通知。**自動リトライしない** |
| 送信検証タイムアウト | `failed` として記録、通知。手動確認を促す |
| Messages.app 未起動 | AppleScript が自動起動を試みる。初回のみ検証タイムアウトを 60 秒に延長 |
| chat.db がロック中 | 次のポーリングで再試行（read-only 接続なので通常は発生しない） |

---

## 12. ロギング

- `tracing` を使用。`~/Library/Logs/net.votepurchase.momreply/app.log` にローテーション付きで出力
- **API キー、メッセージ本文の全文をログレベル info 以上に出さない**（本文は app.db に持つ）
- `debug` レベルでのみ本文をログに出す。デフォルトは `info`
- 90 日で自動削除

---

## 13. 実装フェーズと受け入れ基準

### Phase 0: chat.db 読み取り（CLI）

Tauri を組む前に、単体のバイナリで検証する。ここが通らなければ他は全部無意味。

- [ ] read-only で chat.db に接続できる
- [ ] 母との会話の直近 20 件を取得できる
- [ ] **`attributedBody` から本文がデコードできる**（`text` が NULL でも本文が出る）
- [ ] 自分の送信（`is_from_me = 1`）も本文が取れる
- [ ] タップバック・システムメッセージが除外できている
- [ ] 日時が正しくローカル時刻で表示される

### Phase 1: 生成 + ドライラン

- [ ] few-shot ペアが 40 組抽出できる
- [ ] プロファイルの叩き台が生成できる
- [ ] 新着検知から生成までが自動で回る
- [ ] 生成結果が `dry_run` として app.db に記録される
- [ ] **生成された文章が「自分が書いたっぽい」と感じられる**（体感評価。ここが不合格なら few-shot とプロンプトを調整する）
- [ ] 3 プロバイダすべてで生成できる
- [ ] Swift サイドカーが起動し、`availability` と `contextSize` を返す
- [ ] Apple Intelligence で分類タスクが動く
- [ ] Apple Intelligence が `.unavailable` の環境で `primary` に自動フォールバックする
- [ ] **初回起動時に過去メッセージが一切処理されない**（app.db を消して再起動し、`processed_messages` が空のままであること）

### Phase 2: 送信 + メニューバー UI

- [ ] AppleScript で自分宛のテストメッセージを送信できる
- [ ] 送信成否が chat.db で検証できる
- [ ] トレイからポップオーバーが開く
- [ ] 設定画面から API キーを入力・保存でき、疎通テストの結果が表示される
- [ ] キーがフロントエンドに返っていない（DevTools のネットワーク／IPC を見て確認）
- [ ] キー未設定の状態では自動送信トグルが無効化される
- [ ] 返信案を編集して送信できる
- [ ] 追加指示つき再生成ができる
- [ ] 通知が出て、タップでポップオーバーが開く

### Phase 3: ガード + 全自動

- [ ] Allowlist 外のメッセージが一切処理されない
- [ ] 既返信チェックが機能する（iPhone で先に返信 → スキップされる）
- [ ] スリープ復帰後、溜まったメッセージが一斉送信されない
- [ ] stale guard が機能する
- [ ] レートリミットが機能する
- [ ] **連続 N 回で自動送信が止まり、確認モードに切り替わる**
- [ ] 手動送信・編集でカウンタが 0 に戻る
- [ ] `session_gap_minutes` 経過でカウンタが 0 に戻る
- [ ] 月次ハード上限に達すると自動送信と生成が止まる
- [ ] 推定コストが設定画面に表示される
- [ ] 長さプリセットを切り替えると出力の長さが実際に変わる（特に `very_long` で few-shot に負けていないこと）
- [ ] 長文プリセットでも文体が丁寧語・ビジネス調に崩れていない
- [ ] キルスイッチが 1 クリックで効く
- [ ] ドライランを 1〜2 週間運用し、ログを見て問題がないことを確認してから `dry_run = false`

### Phase 4: プロファイル自動更新

- [ ] 更新候補が抽出される
- [ ] 差分 UI で項目ごとに承認・却下できる
- [ ] 承認分のみ profile.md に反映される
- [ ] 履歴から巻き戻せる

---

## 14. 実装時の注意（既知の落とし穴）

1. **`message.text` はほぼ常に NULL**。ここを見て「メッセージが取れない」と悩む時間が一番長くなる。最初に `attributedBody` のデコードを通すこと
2. **`handle` テーブル経由の JOIN では自分の送信を取りこぼす**。`chat_message_join` → `chat` 経由にすること
3. **AppleScript は送信失敗を報告しないことがある**。終了コード 0 を信じず chat.db で検証すること
4. **フルディスクアクセスは付与後にアプリ再起動が必要**。開発中に「権限を付けたのに読めない」となったらまず再起動
5. **開発中はターミナル / IDE にもフルディスクアクセスが要る**（`cargo run` で起動する場合）
6. **AppleScript の文字列連結は絶対にしない**。改行・引用符・絵文字で壊れる。`argv` 経由で渡すこと
7. **`imessage-database` クレートの API は変わりやすい**。実装開始時にドキュメントで現行シグネチャを確認すること
8. **長文プリセットは few-shot に負ける**。「600文字で書け」と指示しても、文例が全部一言なら一言が返ってくる。6.9.4 の対策を順に試すこと
9. **長文を指示すると文体が丁寧語に寄る**。モデルは「長く書く」を「きちんと書く」と解釈しがち。長さ指示と同時に「文体は崩さない」を必ず併記する
10. **連続カウンタのリセット条件を取りこぼさない**。特に「日付が変わった」を忘れると、翌朝の最初のメッセージが確認モードのまま放置される
11. **開発中はフルディスクアクセスが再ビルドのたびに外れる**。dev ビルドは署名が毎回変わるため、一度付与した FDA が無効になる。対処は次のいずれか。(a) `tauri.conf.json` で ad-hoc 署名の identity を固定する、(b) ターミナル / IDE 側に FDA を付与し、そこから `cargo tauri dev` を起動して権限を継承させる。**(b) のほうが確実**。「権限を付けたのに読めない」となったら、まず FDA のリストから該当エントリを削除して付与し直し、アプリを再起動すること
12. **Apple Intelligence の可用性は環境依存**。`SystemLanguageModel.availability` が `.unavailable` を返すケース（未有効・非対応ハード・リージョン制限・モデル DL 中）を必ずハンドリングする。開発機で動いたから動く、と考えない
13. **Keychain のアイテムはコード署名に紐づく**。dev ビルドで毎回アクセス許可ダイアログが出るのは正常。7.5.7 の対処を先に読むこと
14. **`contextSize` をハードコードしない**。ドキュメント上は 4,096 だが、新しいハードウェアやモデル更新で変わり得る。必ず実行時に読む
8. **テストは自分の Apple ID 宛に送って行う**。母への実送信でデバッグしない

---

## 15. 将来拡張（v1 では実装しない）

- 母以外の相手への対応（`target.handles` を配列にしてあるので、プロファイルと few-shot を相手ごとに分ければ拡張可能）
- **macOS 27 での Apple Intelligence 全面移行**（7.3.5 参照）。`PrivateCloudComputeLanguageModel` の 32K コンテキストで返信生成そのものを移せるか再評価する。実現すれば API 課金がゼロになり、会話が端末外に出なくなる
- カレンダー連携（予定を聞かれたときに実際の空きを見て答える）
- 添付画像の内容理解（vision モデルで写真にコメントする）
- 送信タイミングの学習（普段の返信レイテンシを学んで自然な間隔で送る）
- 週次サマリ（「今週、母から3回連絡があり、うち1回は通院の話でした」）
