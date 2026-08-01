import { useCallback, useEffect, useState } from "react";
import { listKeyStatuses, listModels, type KeyStatus, type ModelSetting } from "./api";
import ApiKeyRow from "./components/ApiKeyRow";
import SelfProfile from "./components/SelfProfile";

export default function App() {
  const [statuses, setStatuses] = useState<KeyStatus[] | null>(null);
  const [models, setModels] = useState<ModelSetting[]>([]);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [s, m] = await Promise.all([listKeyStatuses(), listModels()]);
      setStatuses(s);
      setModels(m);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const anyVerified = statuses?.some((s) => s.verified) ?? false;

  return (
    <div className="flex h-full flex-col text-sm">
      <header className="flex items-center justify-between border-b border-neutral-200 px-4 py-3 dark:border-neutral-700">
        <h1 className="font-semibold">MomReply</h1>
        <span className="text-xs text-neutral-400">設定</span>
      </header>

      <main className="flex-1 overflow-y-auto">
        <SelfProfile />

        <section className="border-t border-neutral-200 dark:border-neutral-700">
          <h2 className="px-4 pt-4 pb-2 text-xs font-semibold tracking-wide text-neutral-500 uppercase dark:text-neutral-400">
            APIキー
          </h2>

          {error && <p className="px-4 pb-2 text-xs text-red-600">{error}</p>}

          {statuses === null ? (
            <p className="px-4 py-3 text-xs text-neutral-400">読み込み中…</p>
          ) : (
            statuses.map((s) => (
              <ApiKeyRow
                key={s.provider}
                status={s}
                model={models.find((m) => m.provider === s.provider)}
                onChange={(next) =>
                  setStatuses((prev) =>
                    (prev ?? []).map((x) => (x.provider === next.provider ? next : x)),
                  )
                }
                onModelChange={(provider, model) =>
                  setModels((prev) =>
                    prev.map((m) =>
                      m.provider === provider ? { ...m, model, customized: true } : m,
                    ),
                  )
                }
              />
            ))
          )}

          <div className="border-b border-neutral-200 px-4 py-3 dark:border-neutral-700">
            <div className="flex items-center justify-between">
              <span className="text-sm font-medium">Apple Intelligence</span>
              <span className="text-xs text-neutral-400">キー不要</span>
            </div>
            <p className="mt-1 text-xs text-neutral-400">未実装</p>
          </div>
        </section>

        {!anyVerified && (
          <p className="px-4 py-3 text-xs text-neutral-500 dark:text-neutral-400">
            検証済みのキーが 1 つも無い間は、自動送信を有効にできません。
          </p>
        )}
      </main>

      <footer className="border-t border-neutral-200 px-4 py-2 text-xs text-neutral-400 dark:border-neutral-700">
        キーは Keychain に保存され、画面には末尾4文字のみ表示されます。
      </footer>
    </div>
  );
}
