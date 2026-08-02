/**
 * 画面の文言。
 *
 * # なぜ辞書を 1 ファイルにまとめるか
 *
 * 文言が各コンポーネントに散っていると、片方の言語だけ直して
 * もう片方が古いまま残る。並べて置けば、追加も差分も一目で分かる。
 *
 * # 何を訳して、何を訳さないか
 *
 * 訳すのは**画面に出る文字**だけ。コード中のコメントとドキュメントは
 * 日本語のままにしてある。読む相手が違う。
 *
 * 生成される返信そのものは訳さない。相手のメッセージと同じ言語で書く、
 * という指示がプロンプトに入っている（`prompt.rs`）。UI が英語でも、
 * 日本語のやり取りには日本語で返る。
 */

export type Lang = "ja" | "en";

export const LANGS: { id: Lang; label: string }[] = [
  { id: "ja", label: "日本語" },
  { id: "en", label: "English" },
];

/** OS の設定から推定する。保存された設定が無いときの既定値。 */
export function detectLang(): Lang {
  return typeof navigator !== "undefined" && navigator.language.startsWith("ja")
    ? "ja"
    : "en";
}

type Entry = { ja: string; en: string };

const DICT = {
  // 共通
  "app.name": { ja: "MomReply", en: "MomReply" },
  "common.loading": { ja: "読み込み中…", en: "Loading…" },
  "common.save": { ja: "保存", en: "Save" },
  "common.saving": { ja: "保存中…", en: "Saving…" },
  "common.saved": { ja: "保存済み", en: "Saved" },
  "common.cancel": { ja: "やめる", en: "Cancel" },
  "common.delete": { ja: "削除する", en: "Delete" },
  "common.recheck": { ja: "もう一度確認する", en: "Check again" },
  "common.generating": { ja: "生成中…", en: "Generating…" },
  "common.unsaved": { ja: "未保存の変更があります", en: "Unsaved changes" },

  // タブ
  "tab.replies": { ja: "返信", en: "Replies" },
  "tab.targets": { ja: "相手", en: "Contacts" },
  "tab.self": { ja: "自分について", en: "About you" },
  "tab.settings": { ja: "設定", en: "Settings" },

  // ヘッダーのバッジ
  "badge.dryRun": { ja: "ドライラン", en: "Dry run" },
  "badge.autoSendOff": { ja: "自動送信OFF", en: "Auto-send off" },
  "badge.noKey": { ja: "APIキー未設定", en: "No API key" },

  // 進行状況
  "activity.settling": {
    ja: "続きが来ないか待っています",
    en: "Waiting in case more arrives",
  },
  "activity.generating": { ja: "返信を作っています", en: "Writing a reply" },

  // フルディスクアクセス
  "access.title": {
    ja: "フルディスクアクセスが必要です",
    en: "Full Disk Access is required",
  },
  "access.body": {
    ja: "MomReply は macOS のメッセージ履歴を読み取り専用で参照します。書き込みは一切しません。この許可が無いと、届いたメッセージを 1 件も読めません。",
    en: "MomReply reads your macOS Messages history in read-only mode. It never writes to it. Without this permission it cannot read a single message.",
  },
  "access.step1": {
    ja: "下のボタンでシステム設定を開く",
    en: "Open System Settings with the button below",
  },
  "access.step2": {
    ja: "一覧に MomReply を追加して、スイッチを入れる",
    en: "Add MomReply to the list and turn it on",
  },
  "access.step3": {
    ja: "MomReply を終了して、開き直す",
    en: "Quit MomReply and open it again",
  },
  "access.restartWarning": {
    ja: "許可したあとは必ず再起動してください。macOS は起動中のアプリに権限を反映しません。",
    en: "You must restart after granting it. macOS does not apply the permission to an app that is already running.",
  },
  "access.openSettings": {
    ja: "システム設定を開く",
    en: "Open System Settings",
  },
  "access.readFrom": { ja: "読み取り先", en: "Reads from" },
  "access.cannotRead": {
    ja: "メッセージを読めません",
    en: "Cannot read messages",
  },

  // 返信タブ
  "replies.empty": {
    ja: "確認待ちの返信はありません。",
    en: "Nothing waiting for review.",
  },
  "replies.noTargets": {
    ja: "相手タブで返信する相手を登録してください。",
    en: "Add someone to reply to in the Contacts tab.",
  },
  "replies.draftLatest": {
    ja: "直近の受信に返信を作る",
    en: "Draft a reply to the latest message",
  },
  "replies.takesTime": {
    ja: "数十秒かかることがあります。",
    en: "This can take up to a minute.",
  },
  "replies.noHistory": { ja: "やり取りがありません。", en: "No messages yet." },
  "replies.context": {
    ja: "生成が読んだ会話",
    en: "Conversation the model read",
  },
  "replies.contextCount": { ja: "（{n}件）", en: " ({n})" },
  "replies.contextNone": {
    ja: "この前のやり取りはありません。",
    en: "Nothing before this message.",
  },
  "replies.contextNote": {
    ja: "返信案を作るときに、これがそのまま LLM へ渡っています。",
    en: "This is passed to the model as-is when drafting.",
  },
  "replies.draftPlaceholder": { ja: "返信案", en: "Draft reply" },
  "replies.instructionPlaceholder": {
    ja: "AIへの指示（任意）",
    en: "Instruction for the model (optional)",
  },
  "replies.regenerate": { ja: "再生成", en: "Regenerate" },
  "replies.send": { ja: "送信 ⌘↵", en: "Send ⌘↵" },
  "replies.skip": { ja: "返さない", en: "Don't reply" },
  "replies.busy.regen": {
    ja: "返信案を生成しています…",
    en: "Generating a draft…",
  },
  "replies.busy.send": {
    ja: "送信して結果を確認しています…",
    en: "Sending and verifying…",
  },
  "replies.busy.skip": { ja: "処理しています…", en: "Working…" },
  "replies.willReplace": {
    ja: "数秒かかります。完了すると案が入れ替わります。",
    en: "This takes a few seconds. The draft will be replaced.",
  },
  "replies.me": { ja: "自分", en: "You" },
  "replies.them": { ja: "相手", en: "Them" },

  // 相手タブ
  "targets.addFirst": { ja: "+ 相手を追加", en: "+ Add a contact" },
  "targets.addAnother": { ja: "+ 別の相手を追加", en: "+ Add another contact" },
  "targets.none": {
    ja: "返信する相手がまだ選ばれていません。",
    en: "No contact selected yet.",
  },
  "targets.pickChat": {
    ja: "会話を選ぶ（本文は読み込みません）",
    en: "Pick a conversation (message bodies are not read)",
  },
  "targets.pickPlaceholder": { ja: "選択してください", en: "Select…" },
  "targets.namePlaceholder": { ja: "表示名", en: "Display name" },
  "targets.backlogNote": {
    ja: "登録した時点より前のメッセージは処理されません。過去分に一斉返信する事故を防ぐためです。",
    en: "Messages received before you add the contact are never processed. This prevents mass-replying to old history.",
  },
  "targets.register": { ja: "登録", en: "Add" },
  "targets.messageCount": { ja: "{n}件", en: "{n} msgs" },
  "targets.fewshot": { ja: "文体の手本 {n} 組", en: "{n} style examples" },
  "targets.fewshotNone": {
    ja: "（このままだと文体が再現されません）",
    en: " (your voice will not be reproduced)",
  },
  "targets.rebuild": { ja: "作り直す", en: "Rebuild" },
  "targets.removeWarning": {
    ja: "{name} を削除すると、処理履歴と文体の手本もまとめて消えます。元に戻せません。",
    en: "Deleting {name} also removes its history and style examples. This cannot be undone.",
  },
  "targets.rename": { ja: "名前を変える", en: "Rename" },
  "targets.renameNote": {
    ja: "この名前がそのまま生成に使われます（「〇〇からの iMessage に返信を書きます」）。",
    en: "This name is used in the prompt as-is (\"writing a reply to X\").",
  },
  "targets.length": { ja: "長さ", en: "Length" },
  "targets.targetChars": { ja: "目標文字数", en: "Target length" },
  "targets.charsUnit": { ja: "文字", en: "chars" },
  "targets.charsHintEmpty": {
    ja: "入れるとプリセットより優先されます（{min}〜{max}）。",
    en: "Overrides the preset when set ({min}–{max}).",
  },
  "targets.charsHintSet": {
    ja: "空にするとプリセットに戻ります。",
    en: "Clear it to go back to the preset.",
  },
  "targets.counters": {
    ja: "連続 {consecutive} / {maxConsecutive} ・ 1時間 {hour} / {maxHour} ・ 24時間 {day} / {maxDay}",
    en: "Streak {consecutive}/{maxConsecutive} · 1h {hour}/{maxHour} · 24h {day}/{maxDay}",
  },
  "targets.resetCounters": { ja: "カウントを0に戻す", en: "Reset counters" },
  "targets.resetNote": {
    ja: "送信履歴は消えません。数え直す起点が動くだけです。",
    en: "Send history is kept. Only the counting restarts.",
  },
  "targets.resetDone": {
    ja: "カウントを 0 に戻しました。",
    en: "Counters reset to 0.",
  },
  "targets.atLimit": {
    ja: "上限に達しています。自動送信は止まり、確認待ちに溜まります。",
    en: "At the limit. Auto-send has stopped; replies will wait for review.",
  },
  "targets.autoSend": { ja: "自動で送信する", en: "Send automatically" },
  "targets.rebuildDone": {
    ja: "文体の手本を {n} 組作りました。",
    en: "Built {n} style examples.",
  },
  "targets.limitsTitle": { ja: "暴走を止める上限", en: "Runaway limits" },
  "targets.limitsNote": {
    ja: "放置して使う場合、ここが最後の歯止めになります。",
    en: "When running unattended, these are the last line of defence.",
  },
  "targets.limit.consecutive": {
    ja: "連続で自動返信する上限",
    en: "Max consecutive auto-replies",
  },
  "targets.limit.perHour": { ja: "1時間あたりの上限", en: "Max per hour" },
  "targets.limit.perDay": { ja: "24時間あたりの上限", en: "Max per day" },
  "targets.limitHint": {
    ja: "これを超えると確認モードに落ちます。放置運用ではここが最初に効きます。",
    en: "Going over drops into review mode. When running unattended this is the first one you hit.",
  },
  "targets.limit.stale": {
    ja: "古すぎる受信とみなす分数",
    en: "Treat as stale after (minutes)",
  },

  // 長さプリセット
  "length.short": { ja: "短め", en: "Short" },
  "length.mirror": { ja: "合わせる", en: "Mirror" },
  "length.normal": { ja: "ふつう", en: "Normal" },
  "length.long": { ja: "長め", en: "Long" },
  "length.very_long": { ja: "かなり長め", en: "Very long" },

  // 自分について
  "self.title": { ja: "自分について", en: "About you" },
  "self.lead": {
    ja: "文章の方向性を指示できます（例:「デスマス調にしない」「絵文字を使わない」）。指示は文体の手本より優先されます。事実を書けば、それだけは言い切ります。",
    en: 'Give direction on how replies should be written (e.g. "keep it casual", "no emoji"). Direction beats the style examples. Facts written here are the only things stated outright.',
  },
  "self.placeholder": {
    ja: "- デスマス調にしない\n- 絵文字は使わない\n- 長く書きすぎない",
    en: "- keep it casual\n- no emoji\n- don't write too much",
  },
  "self.sentToLlm": {
    ja: "この内容は返信生成のたびに LLM へ送られます。外部に出て困ることは書かないでください。",
    en: "This is sent to the model on every generation. Don't write anything you would not want to leave your machine.",
  },
  "self.candidates": {
    ja: "追記候補 {n} 件（承認するまで反映されません）",
    en: "{n} suggested additions (not applied until approved)",
  },
  "self.approve": { ja: "承認", en: "Approve" },
  "self.reject": { ja: "却下", en: "Reject" },
  "self.candidateError": {
    ja: "追記候補を読めません: {reason}",
    en: "Cannot load suggestions: {reason}",
  },
  "self.readError": {
    ja: "self.md を読めません: {reason}",
    en: "Cannot read self.md: {reason}",
  },

  // 設定
  "settings.mode": { ja: "動作モード", en: "Mode" },
  "settings.dryRun": {
    ja: "ドライラン（生成するが送信しない）",
    en: "Dry run (generate but never send)",
  },
  "settings.allowAutoSend": {
    ja: "自動送信を許可する",
    en: "Allow automatic sending",
  },
  "settings.dryRunNote": {
    ja: "ドライラン中は自動送信されません。確認して手で送ることはできます。",
    en: "Nothing is sent automatically while dry run is on. You can still review and send by hand.",
  },
  "settings.needVerifiedKey": {
    ja: "検証済みのキーが必要です。",
    en: "A verified key is required.",
  },
  "settings.perTargetNote": {
    ja: "相手ごとの設定でも自動送信を切れます。",
    en: "Auto-send can also be turned off per contact.",
  },
  "settings.autostart": {
    ja: "ログイン時に自動で起動する",
    en: "Start automatically at login",
  },
  "settings.autostartNote": {
    ja: "入れておかないと、Mac を再起動するたびに手で立ち上げ直すことになります。",
    en: "Without this you have to launch it by hand after every restart.",
  },
  "settings.provider": { ja: "生成に使うAI", en: "Model provider" },
  "settings.providerNote": {
    ja: "返信の生成とプロファイル抽出に使われます。",
    en: "Used for generating replies and extracting facts.",
  },
  "settings.apiKeys": { ja: "APIキー", en: "API keys" },
  "settings.keyNote": {
    ja: "キーは Keychain に保存され、画面には末尾4文字のみ表示されます。",
    en: "Keys are stored in the Keychain. Only the last 4 characters are shown.",
  },
  "settings.noVerifiedKey": {
    ja: "検証済みのキーが 1 つも無い間は、自動送信を有効にできません。",
    en: "Auto-send cannot be enabled until at least one key is verified.",
  },
  "settings.appleUnimplemented": { ja: "未実装", en: "Not implemented" },
  "settings.noKeyNeeded": { ja: "キー不要", en: "No key needed" },
  "settings.version": { ja: "バージョン", en: "Version" },
  "settings.checkUpdate": { ja: "更新を確認", en: "Check for updates" },
  "settings.checking": { ja: "確認中…", en: "Checking…" },
  "settings.upToDate": { ja: "最新です。", en: "Up to date." },
  "settings.updateFound": {
    ja: "新しい版 {version} があります。",
    en: "Version {version} is available.",
  },
  "settings.install": { ja: "入れて再起動", en: "Install and restart" },
  "settings.installing": { ja: "入れています…", en: "Installing…" },
  "settings.updateNote": {
    ja: "更新は署名を検証してから適用されます。署名の無いものは入りません。",
    en: "Updates are verified against a signature before being applied. Unsigned builds are rejected.",
  },
  "settings.language": { ja: "表示言語", en: "Language" },
  "settings.languageNote": {
    ja: "画面の文言だけが変わります。返信は相手のメッセージと同じ言語で書かれます。",
    en: "Only the interface changes. Replies are written in the same language as the incoming message.",
  },

  // API キー行
  "key.verified": { ja: "検証済み", en: "Verified" },
  "key.unverified": { ja: "未検証", en: "Not verified" },
  "key.notSet": { ja: "未設定", en: "Not set" },
  "key.placeholder": { ja: "キーを貼り付け", en: "Paste key" },
  "key.saveAndVerify": { ja: "保存して確認", en: "Save and verify" },
  "key.verify": { ja: "再検証", en: "Verify again" },
  "key.verifying": { ja: "検証中…", en: "Verifying…" },
  "key.delete": { ja: "削除", en: "Delete" },
  "key.change": { ja: "変更", en: "Change" },
  "key.model": { ja: "モデル", en: "Model" },
  "key.modelDefault": { ja: "既定に戻す", en: "Reset to default" },
} as const satisfies Record<string, Entry>;

export type Key = keyof typeof DICT;

/** `{name}` を置き換える。 */
export function translate(
  lang: Lang,
  key: Key,
  vars?: Record<string, string | number>,
): string {
  const raw = DICT[key][lang];
  if (!vars) return raw;
  return raw.replace(/\{(\w+)\}/g, (whole, name: string) =>
    name in vars ? String(vars[name]) : whole,
  );
}
