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
