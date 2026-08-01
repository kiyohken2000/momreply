# MomReply

特定の相手から届く iMessage に、AI が返信文を生成する macOS 常駐アプリ。

生成した返信は、確認したうえで送るか、条件を満たせば自動で送る。
返信対象は chat.db の会話一覧から任意に選べる。

> **状態: 開発中。** 送信機能と UI はまだ無い。
> 現時点で動くのは chat.db の読み取り、返信対象の登録、質問の抽出まで。

---

## これは何を解く道具か

家族や近しい相手とのやり取りで、

- 相手の質問に答えないままにすると関係がこじれる
- しかし毎回文面を考えるのが負担になっている

という状態を想定している。返信を代筆させて負担を下げつつ、
**答えるべきことにはちゃんと答える**ことを目的にする。

そのため「雑談を無難に続ける」より「**質問に具体的に答える**」を優先した設計になっている。
答える材料が無い質問は、勝手に推測させず人間に一度だけ確認し、以後は再利用する。

---

## 動作要件

| | |
|---|---|
| OS | macOS（開発・検証は macOS 26.6） |
| Rust | 1.95 以降（`libsqlite3-sys` が要求する） |
| 権限 | フルディスクアクセス |

Rust は rustup で入れる。Homebrew の `rust` では古い場合がある。

```sh
brew install rustup
rustup default stable
```

`rust-toolchain.toml` で stable に固定してあるので、リポジトリ内では自動で切り替わる。

### フルディスクアクセス

chat.db（`~/Library/Messages/chat.db`）の読み取りに必要。

1. システム設定 → プライバシーとセキュリティ → フルディスクアクセス
2. **`cargo` を起動するアプリ**（ターミナル / VS Code など）を追加して ON
3. **そのアプリを完全に終了して再起動**（ウィンドウを閉じるだけでは反映されない）

付与しないと `unable to open database file` で止まる。

---

## 使い方

```sh
cargo build
```

### 1. 会話相手を確認する

```sh
cargo run -p momreply-cli -- chats
```

本文は読まず、`chat_identifier` と件数・最終日時だけを出す。

### 2. 返信対象を登録する

```sh
cargo run -p momreply-cli -- target add \
  --slug someone --name 表示名 \
  --handle someone@icloud.com
```

電話番号と Apple ID で会話が 2 本に分かれている場合は `--handle` を複数指定する。

**登録した時点より前のメッセージは処理対象にならない。**
過去の会話に一斉返信する事故を防ぐため、登録と同時にその時点の最新 ROWID を記録する。

```sh
cargo run -p momreply-cli -- target list
cargo run -p momreply-cli -- target pending --slug someone   # 未処理の新着
```

### 3. 質問を抽出して答える

```sh
cargo run -p momreply-cli -- questions scan --slug someone
cargo run -p momreply-cli -- questions list --slug someone
cargo run -p momreply-cli -- questions answer --id 3 --answer "持っている"
```

答えた内容は `self.md`（後述）に事実として追記され、次から同じ質問が来ても聞かれない。

### メッセージを直接見る

```sh
cargo run -p momreply-cli -- messages --handle someone@icloud.com --limit 20
```

---

## 安全設計

自動送信は事故が起きると取り返しがつかないため、ガードを先に作っている。

- **chat.db には書き込まない。** 接続は `SQLITE_OPEN_READ_ONLY` のみで、
  接続直後に SQLite 自身へ read-only かを問い合わせて二重確認する。
  接続を作る関数は 1 つしかない。
- **登録前のメッセージは処理しない。** 保護はターゲット登録関数の内側にあり、
  引数に chat.db 接続を必須化してあるため、迂回する経路が存在しない。
- **対象外の相手は読み込まない。** allowlist は取得後のフィルタではなく
  SQL の `WHERE` 句で効かせる。
- **自動送信の既定は OFF。**
- 1 つのハンドルを 2 人の対象に登録できない（UNIQUE 制約）。

未実装のガード（レートリミット、既返信チェック、スリープ復帰時の抑制、
コスト上限、キルスイッチ）は送信機能と同時に入れる。仕様は `docs/momreply-spec.md` にある。

---

## 扱うデータ

| 置き場所 | 内容 |
|---|---|
| `~/Library/Messages/chat.db` | **読み取りのみ。** 登録した相手の会話だけを読む |
| `~/Library/Application Support/net.votepurchase.momreply/app.db` | 処理履歴・生成ログ・質問 |
| `.../self.md` | 自分についての事実。AI が断定してよい材料 |
| `.../targets/<slug>.md` | 相手ごとのプロファイル |
| Keychain | API キー |

API キーとメッセージ本文は、設定ファイル・ログ・リポジトリには書かない。
`self.md` と `app.db` はリポジトリの外（Application Support 配下）にあり、
`.gitignore` でも二重に弾いている。

### `self.md`

相手の質問には自分側の事実が要る。「保険証はありますか？」の答えは
相手のプロファイルをいくら充実させても出てこない。

材料が無いまま生成させると「確認してみる」のようなはぐらかしが毎回出る。
それを避けるため、答えられない質問は推測させず人間に一度だけ聞き、
`self.md` に貯めて再利用する。

---

## 構成

```
crates/
├── momreply-core/          コア
│   └── src/
│       ├── imessage/       chat.db（read-only）
│       ├── store.rs        app.db スキーマ・CRUD
│       ├── questions.rs    疑問文の切り出し
│       ├── profile.rs      self.md / 相手プロファイル
│       └── paths.rs        ファイル配置
└── momreply-cli/           検証・管理 CLI
```

UI（Tauri）を載せたあとも、chat.db へのアクセスは `momreply-core::imessage` に一本化する。

---

## 進捗

- [x] **Phase 0** chat.db 読み取り（read-only 接続 / `attributedBody` デコード / 除外判定）
- [ ] **Phase 1** 生成 + ドライラン
  - [x] 返信対象の登録とバックログ保護
  - [x] 質問の抽出と `self.md`
  - [ ] LLM プロバイダ（Claude / Gemini / OpenAI / Apple Intelligence）
  - [ ] few-shot 抽出
  - [ ] 返信生成
- [ ] **Phase 2** 送信 + メニューバー UI
- [ ] **Phase 3** ガード + 全自動
- [ ] **Phase 4** プロファイル自動更新

詳細な仕様と受け入れ基準は `docs/momreply-spec.md`。

---

## 実装上の注意

macOS の chat.db には引っかかりやすい点がいくつかある。

1. **`message.text` はほぼ常に NULL。** 本文は `attributedBody`（typedstream）から取る。
   検証した環境では対象会話の全件が NULL で、`text` からは 1 件も取れなかった。
2. **`handle` テーブル経由の JOIN では自分の送信を取りこぼす。**
   送信メッセージは `handle_id = 0` になることがある。検証した環境では
   自分の送信の約 3 分の 2 が該当した。`chat_message_join` → `chat` 経由で取る。
3. **フルディスクアクセスは付与後にアプリの再起動が必要。**
4. **dev ビルドは署名が毎回変わる**ため、`.app` に付与した権限が外れる。
   開発中はターミナル / IDE 側に権限を付けて、そこから起動するほうが確実。

---

## 留意点

このツールは、相手から見ると**本人が書いた文面として届く**。
受け手が AI と対話していることを知る手段はない。

誰に向けて使うか、そのことを相手に伝えるかは利用者の判断に委ねられる。
既定値は自動送信 OFF・ドライラン ON にしてあり、
最初は生成結果を目で確認しながら運用することを想定している。

---

## ライセンス

未定。
