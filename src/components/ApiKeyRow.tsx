import { useState } from "react";
import {
  deleteApiKey,
  setApiKey,
  setModel,
  verifyApiKey,
  PROVIDER_LABELS,
  type KeyStatus,
  type ModelSetting,
  type ProviderId,
} from "../api";

type Props = {
  status: KeyStatus;
  model: ModelSetting | undefined;
  onChange: (next: KeyStatus) => void;
  onModelChange: (provider: string, model: string) => void;
};

type Phase = "idle" | "saving" | "verifying" | "deleting";

/**
 * プロバイダ 1 件分の行（仕様書 7.5.4）。
 *
 * 入力欄の値は保存に成功した時点で即座に空にする。
 * React の state にキーを残さない。
 */
export default function ApiKeyRow({ status, model, onChange, onModelChange }: Props) {
  const [draft, setDraft] = useState("");
  const [phase, setPhase] = useState<Phase>("idle");
  const [failure, setFailure] = useState<string | null>(null);
  const [modelDraft, setModelDraft] = useState<string | null>(null);

  const provider = status.provider as ProviderId;
  const busy = phase !== "idle";

  async function save() {
    const key = draft.trim();
    if (!key) return;
    setPhase("saving");
    setFailure(null);
    try {
      const next = await setApiKey(provider, key);
      // 成功・失敗にかかわらず入力欄は空にする。
      // 検証に失敗してもキーは保存されている（仕様書 7.5.5）。
      setDraft("");
      onChange(next);
    } catch (e) {
      setFailure(String(e));
    } finally {
      setPhase("idle");
    }
  }

  async function reverify() {
    setPhase("verifying");
    setFailure(null);
    try {
      onChange(await verifyApiKey(provider));
    } catch (e) {
      setFailure(String(e));
    } finally {
      setPhase("idle");
    }
  }

  async function remove() {
    setPhase("deleting");
    setFailure(null);
    try {
      await deleteApiKey(provider);
      onChange({
        provider,
        configured: false,
        verified: false,
        masked: null,
        last_verified_at: null,
        error: null,
      });
    } catch (e) {
      setFailure(String(e));
    } finally {
      setPhase("idle");
    }
  }

  return (
    <div className="border-b border-neutral-200 px-4 py-3 last:border-b-0 dark:border-neutral-700">
      <div className="flex items-center justify-between gap-2">
        <span className="text-sm font-medium">{PROVIDER_LABELS[provider] ?? provider}</span>
        <StatusBadge status={status} />
      </div>

      {status.configured && (
        <div className="mt-1 font-mono text-xs text-neutral-500 dark:text-neutral-400">
          {status.masked}
        </div>
      )}

      {(status.error || failure) && (
        <p className="mt-1 text-xs break-words text-red-600 dark:text-red-400">
          {failure ?? status.error}
        </p>
      )}

      {model && (
        <label className="mt-2 flex items-center gap-2">
          <span className="shrink-0 text-xs text-neutral-500 dark:text-neutral-400">モデル</span>
          <input
            type="text"
            value={modelDraft ?? model.model}
            placeholder={model.default_model}
            spellCheck={false}
            autoCorrect="off"
            autoCapitalize="off"
            onChange={(e) => setModelDraft(e.target.value)}
            onBlur={async () => {
              if (modelDraft === null || modelDraft === model.model) {
                setModelDraft(null);
                return;
              }
              await setModel(provider, modelDraft);
              onModelChange(provider, modelDraft.trim() || model.default_model);
              setModelDraft(null);
            }}
            className="min-w-0 flex-1 rounded border border-neutral-300 px-2 py-1 font-mono text-xs dark:border-neutral-600 dark:bg-neutral-800"
          />
        </label>
      )}

      {status.configured ? (
        <div className="mt-2 flex gap-2">
          <button
            type="button"
            onClick={reverify}
            disabled={busy}
            className="rounded border border-neutral-300 px-2 py-1 text-xs disabled:opacity-50 dark:border-neutral-600"
          >
            {phase === "verifying" ? "検証中…" : "再検証"}
          </button>
          <button
            type="button"
            onClick={remove}
            disabled={busy}
            className="rounded border border-neutral-300 px-2 py-1 text-xs text-red-600 disabled:opacity-50 dark:border-neutral-600 dark:text-red-400"
          >
            削除
          </button>
        </div>
      ) : (
        <form
          className="mt-2 flex gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            void save();
          }}
        >
          <input
            type="password"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder="APIキーを貼り付け"
            autoComplete="off"
            spellCheck={false}
            autoCorrect="off"
            autoCapitalize="off"
            disabled={busy}
            className="min-w-0 flex-1 rounded border border-neutral-300 px-2 py-1 text-xs disabled:opacity-50 dark:border-neutral-600 dark:bg-neutral-800"
          />
          <button
            type="submit"
            disabled={busy || !draft.trim()}
            className="rounded bg-blue-600 px-3 py-1 text-xs text-white disabled:opacity-40"
          >
            {phase === "saving" ? "検証中…" : "保存"}
          </button>
        </form>
      )}
    </div>
  );
}

function StatusBadge({ status }: { status: KeyStatus }) {
  if (!status.configured) {
    return <span className="text-xs text-neutral-400">○ 未設定</span>;
  }
  if (status.verified) {
    return <span className="text-xs text-green-600 dark:text-green-400">● 検証済み</span>;
  }
  return <span className="text-xs text-amber-600 dark:text-amber-400">⚠ 未検証</span>;
}
