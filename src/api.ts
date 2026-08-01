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
export function setApiKey(provider: ProviderId, key: string): Promise<KeyStatus> {
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

export type PendingQuestion = { id: number; question: string };

export type Pending = {
  chat_rowid: number;
  target_slug: string;
  display_name: string;
  received_at: number;
  incoming: string;
  draft: string;
  status: string;
  reason: string | null;
  /** 答える材料が無い質問。あればこれを先に埋める。 */
  questions: PendingQuestion[];
};

export function listPending(): Promise<Pending[]> {
  return invoke("list_pending");
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

/** 質問への答え方。fact のときだけ self.md に書かれる。 */
export type Stance = "fact" | "deflect" | "ignore";

export const STANCES: { id: Stance; label: string; hint: string }[] = [
  { id: "fact", label: "答える", hint: "入力した内容を事実として self.md に保存します" },
  { id: "deflect", label: "ごまかす", hint: "はっきり答えず受け流します" },
  { id: "ignore", label: "触れない", hint: "この質問には触れずに返します" },
];

export function resolveQuestion(
  id: number,
  stance: Stance,
  answer: string | null,
): Promise<void> {
  return invoke("resolve_question", { id, stance, answer });
}

export type RunMode = { auto_send: boolean; dry_run: boolean };

export function getRunMode(): Promise<RunMode> {
  return invoke("get_run_mode");
}

export function setRunMode(autoSend: boolean, dryRun: boolean): Promise<void> {
  return invoke("set_run_mode", { autoSend, dryRun });
}

export const LENGTH_PRESETS = [
  { id: "short", label: "短め" },
  { id: "mirror", label: "合わせる" },
  { id: "normal", label: "ふつう" },
  { id: "long", label: "長め" },
] as const;

/** `self.md` の全文。AI が事実として断定してよい唯一の材料。 */
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
  reply_preset: string;
  /** precise = 具体的に答える / vague = 曖昧に返して人に聞かない */
  reply_mode: string;
};

export function listTargets(): Promise<TargetView[]> {
  return invoke("list_targets");
}

export function updateTarget(
  slug: string,
  patch: { autoSend?: boolean; replyPreset?: string; replyMode?: string },
): Promise<void> {
  return invoke("update_target", {
    slug,
    autoSend: patch.autoSend ?? null,
    replyPreset: patch.replyPreset ?? null,
    replyMode: patch.replyMode ?? null,
  });
}

export type Limits = {
  max_consecutive_auto: number;
  max_per_hour: number;
  max_per_day: number;
  stale_threshold_minutes: number;
  monthly_soft_limit_usd: number;
  monthly_hard_limit_usd: number;
  month_cost_usd: number;
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
