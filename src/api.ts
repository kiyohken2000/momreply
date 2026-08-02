import { invoke } from "@tauri-apps/api/core";

/**
 * API キーの状態。
 *
 * **キー本体はここに来ない。** `masked` は末尾4文字のみで、
 * 先頭は含まれない（仕様書 7.5.2）。バックエンドにもキーを返す
 * コマンドは存在しない。
 */
export type KeyStatus = {
  provider: string;
  configured: boolean;
  verified: boolean;
  masked: string | null;
  last_verified_at: number | null;
  error: string | null;
};

export type ProviderId = "anthropic" | "gemini" | "openai";

export const PROVIDER_LABELS: Record<ProviderId, string> = {
  anthropic: "Anthropic",
  gemini: "Gemini",
  openai: "OpenAI",
};

export function listKeyStatuses(): Promise<KeyStatus[]> {
  return invoke("list_key_statuses");
}

/** 保存して疎通テストまで行う（仕様書 7.5.5）。 */
export function setApiKey(
  provider: ProviderId,
  key: string,
): Promise<KeyStatus> {
  return invoke("set_api_key", { provider, key });
}

/** 保存済みキーで再テスト。キーは渡さない。 */
export function verifyApiKey(provider: ProviderId): Promise<KeyStatus> {
  return invoke("verify_api_key", { provider });
}

export function deleteApiKey(provider: ProviderId): Promise<void> {
  return invoke("delete_api_key", { provider });
}

export function canEnableAutoSend(): Promise<boolean> {
  return invoke("can_enable_auto_send");
}

/** いま動いているアプリの版。 */
export function appVersion(): Promise<string> {
  return invoke("app_version");
}

export type UpdateInfo = {
  available: boolean;
  version: string;
  notes: string | null;
};

/** GitHub Releases を見る。署名を検証してから受け入れる。 */
export function checkUpdate(): Promise<UpdateInfo> {
  return invoke("check_update");
}

/** 入れて起動し直す。**成功すると戻ってこない。** */
export function installUpdate(): Promise<void> {
  return invoke("install_update");
}

/** ログイン時の自動起動。状態は macOS の LaunchAgent が持つ。 */
export function getAutostart(): Promise<boolean> {
  return invoke("get_autostart");
}

export function setAutostart(enabled: boolean): Promise<void> {
  return invoke("set_autostart", { enabled });
}

/** 保存された表示言語。未設定なら空文字。 */
export function getUiLanguage(): Promise<string> {
  return invoke("get_ui_language");
}

/** Rust 側の通知とツールチップも同じ値を読む。 */
export function setUiLanguage(lang: string): Promise<void> {
  return invoke("set_ui_language", { lang });
}

/** chat.db を読めるか。フルディスクアクセスの有無がここに出る。 */
export type ChatDbStatus = {
  ok: boolean;
  path: string;
  reason: string | null;
  needs_full_disk_access: boolean;
};

export function chatDbStatus(): Promise<ChatDbStatus> {
  return invoke("chat_db_status");
}

/** システム設定のフルディスクアクセスを開く。 */
export function openFullDiskAccessSettings(): Promise<void> {
  return invoke("open_full_disk_access_settings");
}

export type Pending = {
  chat_rowid: number;
  target_slug: string;
  display_name: string;
  received_at: number;
  incoming: string;
  draft: string;
  status: string;
  reason: string | null;
};

export function listPending(): Promise<Pending[]> {
  return invoke("list_pending");
}

/** いま裏で何をしているか。していなければ null。 */
export type Activity = {
  who: string;
  /** 文言は i18n（`activity.<phase>`）にある。 */
  phase: "settling" | "generating";
};

/**
 * 開いた時点の状態。合図（`momreply://activity`）だけでは、
 * 開く前に始まった処理を拾えない。
 */
export function currentActivity(): Promise<Activity | null> {
  return invoke("current_activity");
}

/** 会話の 1 行（古い順に並ぶ）。 */
export type Turn = { from_me: boolean; body: string; at: number };

/**
 * 返信案を作るときに読んだ会話。
 * chat.db から都度読むので、確認するまでの新しいやり取りも含まれる。
 */
export function conversation(chatRowid: number): Promise<Turn[]> {
  return invoke("conversation", { chatRowid });
}

/**
 * 相手との直近のやり取り。確認待ちが無いときに、何が起きたかを見るため。
 * 自動送信された返信もここに現れる。
 */
export function recentConversation(slug: string, limit = 12): Promise<Turn[]> {
  return invoke("recent_conversation", { slug, limit });
}

/** 送信直前の既返信チェックはバックエンドで行われる。 */
export function sendReply(chatRowid: number, text: string): Promise<string> {
  return invoke("send_reply", { chatRowid, text });
}

export function regenerate(
  chatRowid: number,
  instruction: string | null,
  length: string | null,
): Promise<string> {
  return invoke("regenerate", { chatRowid, instruction, length });
}

export function skipPending(chatRowid: number): Promise<void> {
  return invoke("skip_pending", { chatRowid });
}

export type RunMode = { auto_send: boolean; dry_run: boolean };

export function getRunMode(): Promise<RunMode> {
  return invoke("get_run_mode");
}

export function setRunMode(autoSend: boolean, dryRun: boolean): Promise<void> {
  return invoke("set_run_mode", { autoSend, dryRun });
}

/** 表示名は i18n（`length.<id>`）にある。 */
export const LENGTH_PRESETS = [
  "short",
  "mirror",
  "normal",
  "long",
  "very_long",
] as const;

/** 目標文字数の指定は `chars:400` の形で reply_preset に入る。 */
export const CHARS_PREFIX = "chars:";
export const MIN_TARGET_CHARS = 10;
export const MAX_TARGET_CHARS = 2000;

/** プリセットではなく目標文字数が選ばれているなら、その文字数。 */
export function targetChars(replyPreset: string): number | null {
  if (!replyPreset.startsWith(CHARS_PREFIX)) return null;
  const n = Number(replyPreset.slice(CHARS_PREFIX.length));
  return Number.isFinite(n) ? n : null;
}

/**
 * `self.md` の全文。
 *
 * 書き方の指示（「デスマス調にしない」など）と、言い切ってよい事実の置き場。
 * 指示は文例より優先される。
 */
export function getSelfProfile(): Promise<string> {
  return invoke("get_self_profile");
}

export function setSelfProfile(content: string): Promise<void> {
  return invoke("set_self_profile", { content });
}

export function selfProfilePath(): Promise<string> {
  return invoke("self_profile_path");
}

export type FactCandidate = {
  id: number;
  section: string;
  content: string;
  confidence: string;
  evidence_ask: string | null;
  evidence_reply: string | null;
};

export function listFactCandidates(): Promise<FactCandidate[]> {
  return invoke("list_fact_candidates");
}

/** 承認すると self.md に追記され、更新後の全文が返る。 */
export function approveFact(id: number): Promise<string> {
  return invoke("approve_fact", { id });
}

export function rejectFact(id: number): Promise<void> {
  return invoke("reject_fact", { id });
}

export type TargetView = {
  slug: string;
  display_name: string;
  handles: string[];
  enabled: boolean;
  auto_send: boolean;
  /** プリセット名か `chars:400`。[`targetChars`] で判別する。 */
  reply_preset: string;
  /** 文体の手本の数。0 だとその人らしさが出ない。 */
  fewshot_count: number;
  /** いま何回続けて自動返信しているか。上限に当たると確認モードに落ちる。 */
  consecutive_auto: number;
  sent_last_hour: number;
  sent_last_day: number;
};

/**
 * 上限のカウントを 0 に戻す。連続・1 時間・24 時間のすべて。
 * 送信履歴は消えない。数え直しの起点が動くだけ。
 */
export function resetCounters(slug: string): Promise<void> {
  return invoke("reset_counters", { slug });
}

/**
 * 直近の受信で返信案を作り、確認待ち（返信タブ）に積む。
 * 必ず確認待ちに入るので、押しただけでは送信されない。
 */
export function draftLatest(slug: string): Promise<string> {
  return invoke("draft_latest", { slug });
}

/** 過去のやり取りから文体の手本を作り直す。 */
export function rebuildFewshot(slug: string): Promise<number> {
  return invoke("rebuild_fewshot", { slug });
}

export function listTargets(): Promise<TargetView[]> {
  return invoke("list_targets");
}

export type ChatChoice = {
  chat_identifier: string;
  service: string;
  display_name: string;
  message_count: number;
  last_message: string | null;
  registered: boolean;
};

/** chat.db の会話一覧。本文は読まれない。 */
export function listChatChoices(limit = 60): Promise<ChatChoice[]> {
  return invoke("list_chat_choices", { limit });
}

/** 登録した時点より前のメッセージは処理対象にならない。 */
export function addTarget(name: string, handles: string[]): Promise<string> {
  return invoke("add_target", { name, handles });
}

/** 履歴・few-shot・質問もまとめて消える。 */
export function removeTarget(slug: string): Promise<void> {
  return invoke("remove_target", { slug });
}

export function updateTarget(
  slug: string,
  patch: { autoSend?: boolean; replyPreset?: string; displayName?: string },
): Promise<void> {
  return invoke("update_target", {
    slug,
    autoSend: patch.autoSend ?? null,
    replyPreset: patch.replyPreset ?? null,
    displayName: patch.displayName ?? null,
  });
}

export type Limits = {
  max_consecutive_auto: number;
  max_per_hour: number;
  max_per_day: number;
  stale_threshold_minutes: number;
  monthly_soft_limit_usd: number;
  monthly_hard_limit_usd: number;
};

export function getLimits(): Promise<Limits> {
  return invoke("get_limits");
}

export function setLimit(key: string, value: number): Promise<void> {
  return invoke("set_limit", { key, value });
}

export type ProviderChoice = {
  id: string;
  label: string;
  configured: boolean;
  verified: boolean;
  implemented: boolean;
  unavailable_reason: string | null;
};

export function listProviders(): Promise<ProviderChoice[]> {
  return invoke("list_providers");
}

export function getPrimaryProvider(): Promise<string> {
  return invoke("get_primary_provider");
}

export function setPrimaryProvider(provider: string): Promise<void> {
  return invoke("set_primary_provider", { provider });
}

export type ModelSetting = {
  provider: string;
  model: string;
  default_model: string;
  customized: boolean;
};

export function listModels(): Promise<ModelSetting[]> {
  return invoke("list_models");
}

/** 空文字を渡すと既定値に戻る。 */
export function setModel(provider: ProviderId, model: string): Promise<void> {
  return invoke("set_model", { provider, model });
}
