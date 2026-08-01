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
