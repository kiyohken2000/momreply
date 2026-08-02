import { useCallback, useEffect, useState } from "react";
import {
  addTarget,
  getLimits,
  listChatChoices,
  listTargets,
  removeTarget,
  rebuildFewshot,
  resetCounters,
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
import { useLang } from "../lang";

/** 相手ごとの設定と、暴走を止める上限（仕様書 6.4.5）。 */
export default function Targets() {
  const { t } = useLang();
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
    return (
      <p className="px-4 py-4 text-xs text-neutral-400">
        {t("common.loading")}
      </p>
    );
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
          {first ? t("targets.addFirst") : t("targets.addAnother")}
        </button>
      ) : (
        <div>
          <div className="text-[11px] text-neutral-500 dark:text-neutral-400">
            {t("targets.pickChat")}
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
            <option value="">{t("targets.pickPlaceholder")}</option>
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
            placeholder={t("targets.namePlaceholder")}
            className="mt-2 w-full rounded border border-neutral-300 px-2 py-1 text-xs dark:border-neutral-600 dark:bg-neutral-800"
          />

          <p className="mt-1 text-[10px] text-neutral-400">
            {t("targets.backlogNote")}
          </p>

          <div className="mt-2 flex gap-2">
            <button
              type="button"
              disabled={!picked || !newName.trim()}
              onClick={() => void submitAdd()}
              className="rounded bg-blue-600 px-3 py-1 text-xs text-white disabled:opacity-40"
            >
              {t("targets.register")}
            </button>
            <button
              type="button"
              onClick={() => setAdding(false)}
              className="text-xs text-neutral-500 dark:text-neutral-400"
            >
              {t("common.cancel")}
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
          {t("targets.none")}
        </p>
      )}

      {targets.length === 0 && (
        <div className="border-b border-neutral-200 px-4 py-3 dark:border-neutral-700">
          {addBlock(true)}
        </div>
      )}

      {targets.map((x) => (
        <section
          key={x.slug}
          className="border-b border-neutral-200 px-4 py-3 dark:border-neutral-700"
        >
          <div className="flex items-baseline justify-between gap-2">
            <DisplayName
              value={x.display_name}
              onChange={(name) => void patch(x.slug, { displayName: name })}
            />
            <button
              type="button"
              onClick={() => setConfirmRemove(x.slug)}
              className="shrink-0 text-[11px] text-neutral-400 hover:text-red-600"
            >
              {t("common.delete")}
            </button>
          </div>
          <div className="text-[11px] break-all text-neutral-400">
            {x.handles.join(", ")}
          </div>

          {/* 手本が無いと文体が再現されない。0 のときは目立たせる。 */}
          <div className="mt-1 flex items-center gap-2">
            <span
              className={
                "text-[11px] " +
                (x.fewshot_count === 0 ? "text-amber-600" : "text-neutral-400")
              }
            >
              {t("targets.fewshot", { n: x.fewshot_count })}
              {x.fewshot_count === 0 && t("targets.fewshotNone")}
            </span>
            <button
              type="button"
              onClick={() =>
                void (async () => {
                  try {
                    const n = await rebuildFewshot(x.slug);
                    setMessage(t("targets.rebuildDone", { n }));
                    await load();
                  } catch (e) {
                    setError(String(e));
                  }
                })()
              }
              className="rounded border border-neutral-300 px-2 py-0.5 text-[10px] dark:border-neutral-600"
            >
              {t("targets.rebuild")}
            </button>
          </div>

          {/* いまどれだけ自動で送っているか。上限に当たって止まったとき、
              ここを見ないと「壊れた」のか「止められた」のか分からない。 */}
          <div className="mt-2 flex items-center gap-2">
            <span className="flex-1 text-[11px] text-neutral-500 dark:text-neutral-400">
              {t("targets.counters", {
                consecutive: x.consecutive_auto,
                maxConsecutive: limits?.max_consecutive_auto ?? "—",
                hour: x.sent_last_hour,
                maxHour: limits?.max_per_hour ?? "—",
                day: x.sent_last_day,
                maxDay: limits?.max_per_day ?? "—",
              })}
            </span>
            <button
              type="button"
              // 0 のときも押せるままにする。押しても何も変わらないが、
              // 灰色にすると「壊れた」と読まれる。実際そう報告された。
              onClick={() =>
                void (async () => {
                  try {
                    await resetCounters(x.slug);
                    setMessage(t("targets.resetDone"));
                    await load();
                  } catch (e) {
                    setError(String(e));
                  }
                })()
              }
              className="rounded border border-neutral-300 px-2 py-0.5 text-[10px] disabled:opacity-40 dark:border-neutral-600"
            >
              {t("targets.resetCounters")}
            </button>
          </div>
          <p className="mt-0.5 text-[10px] text-neutral-400">{t("targets.resetNote")}</p>
          {limits && x.consecutive_auto >= limits.max_consecutive_auto && (
            <p className="mt-1 text-[10px] text-amber-600">
              {t("targets.atLimit")}
            </p>
          )}

          {confirmRemove === x.slug && (
            <div className="mt-2 rounded border border-red-300 bg-red-50 p-2 dark:border-red-800 dark:bg-red-950/40">
              <p className="text-[11px]">
                {t("targets.removeWarning", { name: x.display_name })}
              </p>
              <div className="mt-1 flex gap-2">
                <button
                  type="button"
                  onClick={() => void doRemove(x.slug)}
                  className="rounded bg-red-600 px-2 py-0.5 text-[11px] text-white"
                >
                  {t("common.delete")}
                </button>
                <button
                  type="button"
                  onClick={() => setConfirmRemove(null)}
                  className="text-[11px] text-neutral-500 dark:text-neutral-400"
                >
                  {t("common.cancel")}
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
                  key={p}
                  type="button"
                  onClick={() => void patch(x.slug, { replyPreset: p })}
                  className={
                    "rounded px-2 py-0.5 text-[11px] " +
                    (x.reply_preset === p
                      ? "bg-blue-600 text-white"
                      : "border border-neutral-300 dark:border-neutral-600")
                  }
                >
                  {t(`length.${p}`)}
                </button>
              ))}
            </div>
            <TargetChars
              value={targetChars(x.reply_preset)}
              onChange={(chars) =>
                void patch(x.slug, {
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
              checked={x.auto_send}
              onChange={(e) =>
                void patch(x.slug, { autoSend: e.target.checked })
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
            {t("targets.limitsTitle")}
          </h3>
          <p className="mt-1 text-[10px] text-neutral-400">
            {t("targets.limitsNote")}
          </p>

          <LimitRow
            label={t("targets.limit.consecutive")}
            hint={t("targets.limitHint")}
            value={limits.max_consecutive_auto}
            onChange={(v) => void limit("max_consecutive_auto", v)}
          />
          <LimitRow
            label={t("targets.limit.perHour")}
            value={limits.max_per_hour}
            onChange={(v) => void limit("max_per_hour", v)}
          />
          <LimitRow
            label={t("targets.limit.perDay")}
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
  const { t } = useLang();
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
          {t("targets.targetChars")}
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
        <span className="text-[11px] text-neutral-400">
          {t("targets.charsUnit")}
        </span>
      </div>
      <p className="mt-0.5 text-[10px] text-neutral-400">
        {value === null
          ? t("targets.charsHintEmpty", {
              min: MIN_TARGET_CHARS,
              max: MAX_TARGET_CHARS,
            })
          : t("targets.charsHintSet")}
      </p>
    </div>
  );
}

/**
 * 表示名。**プロンプトにそのまま入る。**
 *
 * 「〇〇からの iMessage に返信を書きます」の〇〇がこれ。会話一覧に名前が
 * 無い相手だと、登録時にメールアドレスが入り、それが人格の呼び名になる。
 *
 * 入力のたびに保存すると、1 文字消した瞬間に空で保存しようとする。
 * 確定は blur と Enter のときだけにする。
 */
function DisplayName({
  value,
  onChange,
}: {
  value: string;
  onChange: (name: string) => void;
}) {
  const { t } = useLang();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);

  useEffect(() => setDraft(value), [value]);

  function commit() {
    setEditing(false);
    const next = draft.trim();
    // 空にはできない。名前が消えるとプロンプトが壊れる。
    if (!next) {
      setDraft(value);
      return;
    }
    if (next !== value) onChange(next);
  }

  if (!editing) {
    return (
      <button
        type="button"
        onClick={() => setEditing(true)}
        title={t("targets.renameNote")}
        className="min-w-0 truncate text-left text-sm font-medium hover:underline"
      >
        {value}
      </button>
    );
  }

  return (
    <input
      type="text"
      autoFocus
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          e.currentTarget.blur();
        }
        if (e.key === "Escape") {
          setDraft(value);
          setEditing(false);
        }
      }}
      className="min-w-0 flex-1 rounded border border-neutral-300 px-1.5 py-0.5 text-sm dark:border-neutral-600 dark:bg-neutral-800"
    />
  );
}
