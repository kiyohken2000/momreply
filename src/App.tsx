import { useCallback, useEffect, useState } from "react";
import {
  getPrimaryProvider,
  getRunMode,
  listKeyStatuses,
  listModels,
  listProviders,
  setPrimaryProvider,
  setRunMode,
  type KeyStatus,
  type ModelSetting,
  type ProviderChoice,
  type RunMode,
} from "./api";
import ApiKeyRow from "./components/ApiKeyRow";
import SelfProfile from "./components/SelfProfile";
import Replies from "./components/Replies";
import Targets from "./components/Targets";

/**
 * ポップオーバーは 380x560 しかない。縦に積むと下のものが画面外へ
 * 押し出されて、存在に気づけなくなる。区切って切り替える。
 */
type Tab = "replies" | "targets" | "self" | "settings";

const TABS: { id: Tab; label: string }[] = [
  { id: "replies", label: "返信" },
  { id: "targets", label: "相手" },
  { id: "self", label: "自分について" },
  { id: "settings", label: "設定" },
];

export default function App() {
  const [tab, setTab] = useState<Tab>("replies");
  const [mode, setMode] = useState<RunMode | null>(null);
  const [statuses, setStatuses] = useState<KeyStatus[] | null>(null);
  const [models, setModels] = useState<ModelSetting[]>([]);
  const [providers, setProviders] = useState<ProviderChoice[]>([]);
  const [primary, setPrimary] = useState<string>("");
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [s, m, r, pv, pr] = await Promise.all([
        listKeyStatuses(),
        listModels(),
        getRunMode(),
        listProviders(),
        getPrimaryProvider(),
      ]);
      setStatuses(s);
      setModels(m);
      setMode(r);
      setProviders(pv);
      setPrimary(pr);
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
          <div className="flex items-center gap-2 text-xs">
            {mode?.dry_run && <span className="text-amber-600">ドライラン</span>}
            {mode && !mode.dry_run && !mode.auto_send && (
              <span className="text-neutral-400">自動送信OFF</span>
            )}
            {statuses !== null && !anyConfigured && (
              <span className="text-amber-600">APIキー未設定</span>
            )}
          </div>
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
        {tab === "replies" && <Replies />}
        {/* 返信案を作ったら返信タブへ送る。作ったものが見えない場所に
            溜まると、確認待ちがあることに気づけない。 */}
        {tab === "targets" && <Targets onDrafted={() => setTab("replies")} />}
        {tab === "self" && <SelfProfile />}

        {tab === "settings" && (
          <section className="h-full overflow-y-auto">
            <h2 className="px-4 pt-4 pb-2 text-xs font-semibold tracking-wide text-neutral-500 uppercase dark:text-neutral-400">
              動作モード
            </h2>
            {mode && (
              <div className="border-b border-neutral-200 px-4 pb-3 dark:border-neutral-700">
                {/* ドライランを先に置く。実際に送るかどうかが最も重要な設定。 */}
                <label className="flex items-center gap-2">
                  <input
                    type="checkbox"
                    checked={mode.dry_run}
                    onChange={(e) => {
                      const next = { ...mode, dry_run: e.target.checked };
                      setMode(next);
                      void setRunMode(next.auto_send, next.dry_run);
                    }}
                  />
                  <span className="text-xs">
                    ドライラン（生成するが<strong>送信しない</strong>）
                  </span>
                </label>
                <label className="mt-2 flex items-center gap-2">
                  <input
                    type="checkbox"
                    checked={mode.auto_send}
                    disabled={mode.dry_run || !anyVerified}
                    onChange={(e) => {
                      const next = { ...mode, auto_send: e.target.checked };
                      setMode(next);
                      void setRunMode(next.auto_send, next.dry_run);
                    }}
                  />
                  <span className="text-xs">自動送信を許可する</span>
                </label>
                <p className="mt-1 text-[11px] text-neutral-400">
                  {mode.dry_run
                    ? "ドライラン中は自動送信されません。確認して手で送ることはできます。"
                    : !anyVerified
                      ? "検証済みのキーが必要です。"
                      : "相手ごとの設定でも自動送信を切れます。"}
                </p>
              </div>
            )}

            <h2 className="px-4 pt-4 pb-2 text-xs font-semibold tracking-wide text-neutral-500 uppercase dark:text-neutral-400">
              生成に使うAI
            </h2>
            <div className="border-b border-neutral-200 px-4 pb-3 dark:border-neutral-700">
              {providers.map((p) => (
                <label
                  key={p.id}
                  className="flex items-start gap-2 py-1"
                  title={p.unavailable_reason ?? ""}
                >
                  <input
                    type="radio"
                    name="primary"
                    className="mt-0.5"
                    checked={primary === p.id}
                    // 使えないものは選ばせない。選べてしまうと、次の受信で
                    // 生成が失敗して初めて気づくことになる。
                    disabled={!p.implemented || !p.configured || !p.verified}
                    onChange={async () => {
                      try {
                        await setPrimaryProvider(p.id);
                        setPrimary(p.id);
                        setError(null);
                      } catch (e) {
                        setError(String(e));
                      }
                    }}
                  />
                  <span className="text-xs">
                    {p.label}
                    {p.unavailable_reason && (
                      <span className="ml-1 text-neutral-400">（{p.unavailable_reason}）</span>
                    )}
                  </span>
                </label>
              ))}
              <p className="mt-1 text-[11px] text-neutral-400">
                返信の生成とプロファイル抽出に使われます。
              </p>
            </div>

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
