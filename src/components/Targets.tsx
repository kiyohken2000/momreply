import { useCallback, useEffect, useState } from "react";
import {
  addTarget,
  getLimits,
  listChatChoices,
  listTargets,
  removeTarget,
  rebuildFewshot,
  setLimit,
  targetChars,
  updateTarget,
  CHARS_PREFIX,
  LENGTH_PRESETS,
  MAX_TARGET_CHARS,
  MIN_TARGET_CHARS,
  type ChatChoice,
  type Limits,
  type TargetView,
} from "../api";

/** 相手ごとの設定と、暴走を止める上限（仕様書 6.4.5）。 */
export default function Targets() {
  const [targets, setTargets] = useState<TargetView[] | null>(null);
  const [limits, setLimits] = useState<Limits | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [chats, setChats] = useState<ChatChoice[] | null>(null);
  const [picked, setPicked] = useState<string>("");
  const [newName, setNewName] = useState("");
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);

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

  /*
   * 相手の追加。会話一覧から選ぶ。手打ちさせない（仕様書 10.2-3）。
   *
   * 1 人登録済みでも消さない。消すと、別の人に切り替えるには一度削除する
   * しかなくなり、処理履歴と文体の手本（数十組）が道連れになる。
   * 代わりに、既に相手がいるときは一覧の**下**へ小さく置く。
   */
  const addBlock = (first: boolean) => (
    <>
      {!adding ? (
        <button
          type="button"
          onClick={() => void openAdd()}
          className={
            // 1 人目はここからしか始められない。まだ誰もいないときだけ目立たせる。
            first
              ? "rounded border border-neutral-300 px-2 py-1 text-xs dark:border-neutral-600"
              : "text-xs text-neutral-500 hover:text-neutral-800 dark:text-neutral-400 dark:hover:text-neutral-100"
          }
        >
          {first ? "+ 相手を追加" : "+ 別の相手を追加"}
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
              const c = chats?.find(
                (x) => x.chat_identifier === e.target.value,
              );
              if (c && !newName)
                setNewName(c.display_name || c.chat_identifier);
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
    </>
  );

  return (
    <div className="h-full overflow-y-auto pb-4">
      {error && (
        <p className="px-4 pt-3 text-xs break-words text-red-600">{error}</p>
      )}
      {message && (
        <p className="px-4 pt-3 text-xs break-words text-green-600">
          {message}
        </p>
      )}

      {targets.length === 0 && !adding && (
        <p className="px-4 py-4 text-xs text-neutral-400">
          返信する相手がまだ選ばれていません。
        </p>
      )}

      {targets.length === 0 && (
        <div className="border-b border-neutral-200 px-4 py-3 dark:border-neutral-700">
          {addBlock(true)}
        </div>
      )}

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
          <div className="text-[11px] break-all text-neutral-400">
            {t.handles.join(", ")}
          </div>

          {/* 手本が無いと文体が再現されない。0 のときは目立たせる。 */}
          <div className="mt-1 flex items-center gap-2">
            <span
              className={
                "text-[11px] " +
                (t.fewshot_count === 0 ? "text-amber-600" : "text-neutral-400")
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
                {t.display_name} を削除すると、
                <strong>処理履歴と文体の手本もまとめて消えます。</strong>
                元に戻せません。
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
            <div className="text-[11px] text-neutral-500 dark:text-neutral-400">
              長さ
            </div>
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
            <TargetChars
              value={targetChars(t.reply_preset)}
              onChange={(chars) =>
                void patch(t.slug, {
                  // 空にしたらプリセットへ戻す。長さの指定が
                  // 何も無い状態は作らない。
                  replyPreset:
                    chars === null ? "mirror" : `${CHARS_PREFIX}${chars}`,
                })
              }
            />
          </div>

          {/* 自動送信は最後に置く。ここを入れると確認なしに本物が飛ぶ。 */}
          <label className="mt-3 flex items-start gap-2">
            <input
              type="checkbox"
              className="mt-0.5"
              checked={t.auto_send}
              onChange={(e) =>
                void patch(t.slug, { autoSend: e.target.checked })
              }
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

      {/* 2 人目以降はここから。一覧の上に置くと、いつも使う相手より
          先に「追加」が目に入る。 */}
      {targets.length > 0 && (
        <div className="border-b border-neutral-200 px-4 py-3 dark:border-neutral-700">
          {addBlock(false)}
        </div>
      )}

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

/**
 * 目標文字数。プリセットより優先される（[`targetChars`]）。
 *
 * 入力のたびに保存すると、「4」と打った瞬間に 4 文字で保存されてしまう。
 * 確定は blur と Enter のときだけにする。
 */
function TargetChars({
  value,
  onChange,
}: {
  value: number | null;
  onChange: (chars: number | null) => void;
}) {
  const [draft, setDraft] = useState(value === null ? "" : String(value));

  // 別の場所でプリセットが選ばれたら、こちらの表示も追従させる。
  useEffect(() => {
    setDraft(value === null ? "" : String(value));
  }, [value]);

  function commit() {
    const trimmed = draft.trim();
    if (trimmed === "") {
      if (value !== null) onChange(null);
      return;
    }
    const n = Math.round(Number(trimmed));
    if (!Number.isFinite(n)) {
      setDraft(value === null ? "" : String(value));
      return;
    }
    const clamped = Math.min(Math.max(n, MIN_TARGET_CHARS), MAX_TARGET_CHARS);
    setDraft(String(clamped));
    if (clamped !== value) onChange(clamped);
  }

  return (
    <div className="mt-2">
      <div className="flex items-center gap-2">
        <span className="flex-1 text-[11px] text-neutral-500 dark:text-neutral-400">
          目標文字数
        </span>
        <input
          type="number"
          min={MIN_TARGET_CHARS}
          max={MAX_TARGET_CHARS}
          step={50}
          value={draft}
          placeholder="—"
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              e.currentTarget.blur();
            }
          }}
          className="w-20 rounded border border-neutral-300 px-2 py-0.5 text-right text-xs dark:border-neutral-600 dark:bg-neutral-800"
        />
        <span className="text-[11px] text-neutral-400">文字</span>
      </div>
      <p className="mt-0.5 text-[10px] text-neutral-400">
        {value === null
          ? `入れるとプリセットより優先されます（${MIN_TARGET_CHARS}〜${MAX_TARGET_CHARS}）。`
          : "空にするとプリセットに戻ります。"}
      </p>
    </div>
  );
}
