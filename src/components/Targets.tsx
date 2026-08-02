import { useCallback, useEffect, useState } from "react";
import {
  addTarget,
  draftLatest,
  getLimits,
  listChatChoices,
  listTargets,
  removeTarget,
  previewReply,
  rebuildFewshot,
  setLimit,
  updateTarget,
  LENGTH_PRESETS,
  type ChatChoice,
  type Preview,
  type Limits,
  type TargetView,
} from "../api";

const MODES = [
  {
    id: "vague",
    label: "おまかせ",
    hint: "明確な答えは出さず、当たり障りのない長文で返します。あなたへの確認は発生しません。",
  },
  {
    id: "precise",
    label: "きちんと答える",
    hint: "質問に具体的に答えます。答える材料が無いときはあなたに聞きます。",
  },
] as const;

/** 相手ごとの設定と、暴走を止める上限（仕様書 6.4.5）。 */
export default function Targets({ onDrafted }: { onDrafted?: () => void }) {
  const [targets, setTargets] = useState<TargetView[] | null>(null);
  const [limits, setLimits] = useState<Limits | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [chats, setChats] = useState<ChatChoice[] | null>(null);
  const [picked, setPicked] = useState<string>("");
  const [newName, setNewName] = useState("");
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [previewing, setPreviewing] = useState<string | null>(null);
  const [drafting, setDrafting] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [t, l] = await Promise.all([listTargets(), getLimits()]);
      setTargets(t);
      setLimits(l);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function patch(slug: string, p: Parameters<typeof updateTarget>[1]) {
    try {
      await updateTarget(slug, p);
      await load();
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function limit(key: string, value: number) {
    try {
      await setLimit(key, value);
      await load();
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function openAdd() {
    setAdding(true);
    setError(null);
    try {
      setChats(await listChatChoices());
    } catch (e) {
      setError(String(e));
    }
  }

  async function submitAdd() {
    try {
      setMessage(await addTarget(newName, [picked]));
      setAdding(false);
      setPicked("");
      setNewName("");
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function doRemove(slug: string) {
    try {
      await removeTarget(slug);
      setConfirmRemove(null);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  if (targets === null) {
    return <p className="px-4 py-4 text-xs text-neutral-400">読み込み中…</p>;
  }

  return (
    <div className="h-full overflow-y-auto pb-4">
      {error && <p className="px-4 pt-3 text-xs break-words text-red-600">{error}</p>}
      {message && <p className="px-4 pt-3 text-xs break-words text-green-600">{message}</p>}

      {targets.length === 0 && !adding && (
        <p className="px-4 py-4 text-xs text-neutral-400">
          返信する相手がまだ選ばれていません。
        </p>
      )}

      {/* 相手の追加。会話一覧から選ぶ。手打ちさせない（仕様書 10.2-3）。 */}
      <div className="border-b border-neutral-200 px-4 py-3 dark:border-neutral-700">
        {!adding ? (
          <button
            type="button"
            onClick={() => void openAdd()}
            className="rounded border border-neutral-300 px-2 py-1 text-xs dark:border-neutral-600"
          >
            + 相手を追加
          </button>
        ) : (
          <div>
            <div className="text-[11px] text-neutral-500 dark:text-neutral-400">
              会話を選ぶ（本文は読み込みません）
            </div>
            <select
              value={picked}
              onChange={(e) => {
                setPicked(e.target.value);
                const c = chats?.find((x) => x.chat_identifier === e.target.value);
                if (c && !newName) setNewName(c.display_name || c.chat_identifier);
              }}
              className="mt-1 w-full rounded border border-neutral-300 px-2 py-1 text-xs dark:border-neutral-600 dark:bg-neutral-800"
            >
              <option value="">選択してください</option>
              {(chats ?? [])
                .filter((c) => !c.registered)
                .map((c) => (
                  <option key={c.chat_identifier} value={c.chat_identifier}>
                    {c.display_name || c.chat_identifier} ({c.message_count}件
                    {c.last_message ? ` / ${c.last_message}` : ""})
                  </option>
                ))}
            </select>

            <input
              type="text"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              placeholder="表示名"
              className="mt-2 w-full rounded border border-neutral-300 px-2 py-1 text-xs dark:border-neutral-600 dark:bg-neutral-800"
            />

            <p className="mt-1 text-[10px] text-neutral-400">
              登録した時点より前のメッセージは処理されません。過去分に一斉返信する
              事故を防ぐためです。
            </p>

            <div className="mt-2 flex gap-2">
              <button
                type="button"
                disabled={!picked || !newName.trim()}
                onClick={() => void submitAdd()}
                className="rounded bg-blue-600 px-3 py-1 text-xs text-white disabled:opacity-40"
              >
                登録
              </button>
              <button
                type="button"
                onClick={() => setAdding(false)}
                className="text-xs text-neutral-500 dark:text-neutral-400"
              >
                やめる
              </button>
            </div>
          </div>
        )}
      </div>

      {targets.map((t) => (
        <section
          key={t.slug}
          className="border-b border-neutral-200 px-4 py-3 dark:border-neutral-700"
        >
          <div className="flex items-baseline justify-between gap-2">
            <span className="text-sm font-medium">{t.display_name}</span>
            <button
              type="button"
              onClick={() => setConfirmRemove(t.slug)}
              className="shrink-0 text-[11px] text-neutral-400 hover:text-red-600"
            >
              削除
            </button>
          </div>
          <div className="text-[11px] break-all text-neutral-400">{t.handles.join(", ")}</div>

          {/* 手本が無いと文体が再現されない。0 のときは目立たせる。 */}
          <div className="mt-1 flex items-center gap-2">
            <span
              className={
                "text-[11px] " + (t.fewshot_count === 0 ? "text-amber-600" : "text-neutral-400")
              }
            >
              文体の手本 {t.fewshot_count} 組
              {t.fewshot_count === 0 && "（このままだと文体が再現されません）"}
            </span>
            <button
              type="button"
              onClick={() =>
                void (async () => {
                  try {
                    const n = await rebuildFewshot(t.slug);
                    setMessage(`文体の手本を ${n} 組作りました。`);
                    await load();
                  } catch (e) {
                    setError(String(e));
                  }
                })()
              }
              className="rounded border border-neutral-300 px-2 py-0.5 text-[10px] dark:border-neutral-600"
            >
              作り直す
            </button>
          </div>

          {confirmRemove === t.slug && (
            <div className="mt-2 rounded border border-red-300 bg-red-50 p-2 dark:border-red-800 dark:bg-red-950/40">
              <p className="text-[11px]">
                {t.display_name} を削除すると、<strong>処理履歴・文体の手本・
                質問への答えもまとめて消えます。</strong>元に戻せません。
              </p>
              <div className="mt-1 flex gap-2">
                <button
                  type="button"
                  onClick={() => void doRemove(t.slug)}
                  className="rounded bg-red-600 px-2 py-0.5 text-[11px] text-white"
                >
                  削除する
                </button>
                <button
                  type="button"
                  onClick={() => setConfirmRemove(null)}
                  className="text-[11px] text-neutral-500 dark:text-neutral-400"
                >
                  やめる
                </button>
              </div>
            </div>
          )}

          <div className="mt-2">
            <div className="text-[11px] text-neutral-500 dark:text-neutral-400">返信の方針</div>
            {MODES.map((m) => (
              <label key={m.id} className="mt-1 flex items-start gap-2">
                <input
                  type="radio"
                  name={`mode-${t.slug}`}
                  className="mt-0.5"
                  checked={t.reply_mode === m.id}
                  onChange={() => void patch(t.slug, { replyMode: m.id })}
                />
                <span className="text-xs">
                  {m.label}
                  <span className="block text-[10px] text-neutral-400">{m.hint}</span>
                </span>
              </label>
            ))}
          </div>

          <div className="mt-2">
            <div className="text-[11px] text-neutral-500 dark:text-neutral-400">長さ</div>
            <div className="mt-1 flex flex-wrap gap-1">
              {LENGTH_PRESETS.map((p) => (
                <button
                  key={p.id}
                  type="button"
                  onClick={() => void patch(t.slug, { replyPreset: p.id })}
                  className={
                    "rounded px-2 py-0.5 text-[11px] " +
                    (t.reply_preset === p.id
                      ? "bg-blue-600 text-white"
                      : "border border-neutral-300 dark:border-neutral-600")
                  }
                >
                  {p.label}
                </button>
              ))}
            </div>
          </div>

          {/* 返信案を確認待ちに積む。必ず人の確認を挟むので、押しても飛ばない。 */}
          <div className="mt-3">
            <button
              type="button"
              disabled={drafting !== null || previewing !== null}
              onClick={() =>
                void (async () => {
                  setDrafting(t.slug);
                  setPreview(null);
                  setError(null);
                  setMessage(null);
                  try {
                    setMessage(await draftLatest(t.slug));
                    onDrafted?.();
                  } catch (e) {
                    setError(String(e));
                  } finally {
                    setDrafting(null);
                  }
                })()
              }
              className="rounded bg-blue-600 px-2 py-1 text-xs text-white disabled:opacity-40"
            >
              {drafting === t.slug ? "生成中…" : "直近の受信に返信を作る"}
            </button>
            <p className="mt-1 text-[10px] text-neutral-400">
              返信タブに入ります。送信するかどうかは、そこで見てから決められます。
            </p>
          </div>

          {/* 試し生成。記録も処理位置も動かさないので、何度押しても安全。 */}
          <div className="mt-3">
            <button
              type="button"
              disabled={previewing !== null || drafting !== null}
              onClick={() =>
                void (async () => {
                  setPreviewing(t.slug);
                  setPreview(null);
                  setError(null);
                  try {
                    setPreview(await previewReply(t.slug));
                  } catch (e) {
                    setError(String(e));
                  } finally {
                    setPreviewing(null);
                  }
                })()
              }
              className="rounded border border-neutral-300 px-2 py-1 text-xs disabled:opacity-40 dark:border-neutral-600"
            >
              {previewing === t.slug ? "生成中…" : "いまの設定で試す"}
            </button>
            <p className="mt-1 text-[10px] text-neutral-400">
              直近の受信メッセージで返信案を作ります。送信も記録もせず、
              処理位置も動かしません。
            </p>

            {preview && previewing === null && (
              <div className="mt-2 rounded border border-neutral-300 p-2 dark:border-neutral-600">
                <div className="text-[10px] text-neutral-400">相手</div>
                <div className="text-[11px] whitespace-pre-wrap">{preview.incoming}</div>
                <div className="mt-2 text-[10px] text-neutral-400">返信案（送信していません）</div>
                <div className="text-[11px] whitespace-pre-wrap">{preview.draft}</div>
                <div className="mt-1 text-[10px] text-neutral-400">
                  {preview.model} / {(preview.latency_ms / 1000).toFixed(1)}秒
                </div>
              </div>
            )}
          </div>

          {/* 自動送信は最後に置く。ここを入れると確認なしに本物が飛ぶ。 */}
          <label className="mt-3 flex items-start gap-2">
            <input
              type="checkbox"
              className="mt-0.5"
              checked={t.auto_send}
              onChange={(e) => void patch(t.slug, { autoSend: e.target.checked })}
            />
            <span className="text-xs">
              自動で送信する
              <span className="block text-[10px] text-amber-600">
                確認なしに本物のメッセージが送られます。全体設定のドライランが
                ONの間は送られません。
              </span>
            </span>
          </label>
        </section>
      ))}

      {limits && (
        <section className="px-4 py-3">
          <h3 className="text-xs font-semibold tracking-wide text-neutral-500 uppercase dark:text-neutral-400">
            暴走を止める上限
          </h3>
          <p className="mt-1 text-[10px] text-neutral-400">
            放置して使う場合、ここが最後の歯止めになります。
          </p>

          <LimitRow
            label="連続で自動返信する上限"
            hint="これを超えると確認モードに落ちます。放置運用ではここが最初に効きます。"
            value={limits.max_consecutive_auto}
            onChange={(v) => void limit("max_consecutive_auto", v)}
          />
          <LimitRow
            label="1時間あたりの送信数"
            value={limits.max_per_hour}
            onChange={(v) => void limit("max_per_hour", v)}
          />
          <LimitRow
            label="1日あたりの送信数"
            value={limits.max_per_day}
            onChange={(v) => void limit("max_per_day", v)}
          />
          <p className="mt-3 text-[10px] text-neutral-400">
            金額での歯止めは効きません（モデルの単価を登録していないため）。
            放置して使う場合、上の 3 つが実質的な上限になります。
          </p>
        </section>
      )}
    </div>
  );
}

function LimitRow({
  label,
  hint,
  value,
  step = 1,
  onChange,
}: {
  label: string;
  hint?: string;
  value: number;
  step?: number;
  onChange: (value: number) => void;
}) {
  const [draft, setDraft] = useState(String(value));
  useEffect(() => setDraft(String(value)), [value]);

  return (
    <div className="mt-2">
      <div className="flex items-center gap-2">
        <span className="flex-1 text-xs">{label}</span>
        <input
          type="number"
          min={0}
          step={step}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() => {
            const n = Number(draft);
            if (Number.isFinite(n) && n >= 0 && n !== value) onChange(n);
            else setDraft(String(value));
          }}
          className="w-20 rounded border border-neutral-300 px-2 py-0.5 text-right text-xs dark:border-neutral-600 dark:bg-neutral-800"
        />
      </div>
      {hint && <p className="text-[10px] text-neutral-400">{hint}</p>}
    </div>
  );
}
