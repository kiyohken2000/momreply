import { useCallback, useEffect, useState } from "react";
import { listKeyStatuses, listModels, type KeyStatus, type ModelSetting } from "./api";
import ApiKeyRow from "./components/ApiKeyRow";
import SelfProfile from "./components/SelfProfile";

/**
 * ポップオーバーは 380x560 しかない。縦に積むと下のものが画面外へ
 * 押し出されて、存在に気づけなくなる。区切って切り替える。
 */
type Tab = "self" | "settings";

const TABS: { id: Tab; label: string }[] = [
  { id: "self", label: "自分について" },
  { id: "settings", label: "設定" },
];

export default function App() {
  const [tab, setTab] = useState<Tab>("self");
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
  const anyConfigured = statuses?.some((s) => s.configured) ?? false;

  return (
    <div className="flex h-full flex-col text-sm">
      <header className="border-b border-neutral-200 dark:border-neutral-700">
        <div className="flex items-center justify-between px-4 pt-3">
          <h1 className="font-semibold">MomReply</h1>
          {/* キーが 1 つも無いことは、設定タブを開かなくても分かるようにする。 */}
          {statuses !== null && !anyConfigured && (
            <span className="text-xs text-amber-600">APIキー未設定</span>
          )}
        </div>
        <nav className="mt-2 flex gap-1 px-2">
          {TABS.map((t) => (
            <button
              key={t.id}
              type="button"
              onClick={() => setTab(t.id)}
              className={
                "rounded-t px-3 py-1.5 text-xs " +
                (tab === t.id
                  ? "border-b-2 border-blue-600 font-medium"
                  : "text-neutral-500 hover:text-neutral-800 dark:text-neutral-400 dark:hover:text-neutral-100")
              }
            >
              {t.label}
            </button>
          ))}
        </nav>
      </header>

      {/* min-h-0 が無いと、中身の高さで main が伸びて
          テキストエリアの flex-1 が効かない。 */}
      <main className="min-h-0 flex-1 overflow-hidden">
        {tab === "self" && <SelfProfile />}

        {tab === "settings" && (
          <section className="h-full overflow-y-auto">
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

            {!anyVerified && (
              <p className="px-4 py-3 text-xs text-neutral-500 dark:text-neutral-400">
                検証済みのキーが 1 つも無い間は、自動送信を有効にできません。
              </p>
            )}

            <p className="px-4 pb-4 text-[11px] text-neutral-400">
              キーは Keychain に保存され、画面には末尾4文字のみ表示されます。
            </p>
          </section>
        )}
      </main>
    </div>
  );
}
