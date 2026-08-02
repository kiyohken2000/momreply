import { useCallback, useEffect, useRef, useState } from "react";
import {
  listPending,
  regenerate,
  sendReply,
  skipPending,
  LENGTH_PRESETS,
  type Pending,
} from "../api";

/** 実行中の操作。何が起きているか分からない時間を作らないために持つ。 */
type Busy = null | "regen" | "send" | "skip";

const BUSY_LABEL: Record<Exclude<Busy, null>, string> = {
  regen: "返信案を生成しています…",
  send: "送信して結果を確認しています…",
  skip: "処理しています…",
};

/**
 * 確認待ちの返信（仕様書 6.6）。
 *
 * # レイアウトの方針
 *
 * 下書きと操作は**常に見える位置に固定する**。受信本文や質問が長いと
 * スクロールで押し出され、書きかけの返信が見えなくなる。
 * スクロールするのは受信内容と質問だけ。
 */
export default function Replies() {
  const [items, setItems] = useState<Pending[] | null>(null);
  const [index, setIndex] = useState(0);
  const [draft, setDraft] = useState("");
  const [instruction, setInstruction] = useState("");
  const [busy, setBusy] = useState<Busy>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const editor = useRef<HTMLTextAreaElement>(null);

  const load = useCallback(async () => {
    try {
      const list = await listPending();
      setItems(list);
      setIndex((i) => Math.min(i, Math.max(0, list.length - 1)));
      setError(null);
    } catch (e) {
      setError(String(e));
      setItems([]);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const current = items?.[index];

  useEffect(() => {
    setDraft(current?.draft ?? "");
    setInstruction("");
    setMessage(null);
  }, [current?.chat_rowid, current?.draft]);

  useEffect(() => {
    if (current) editor.current?.focus();
  }, [current?.chat_rowid]);

  async function run(label: Exclude<Busy, null>, fn: () => Promise<void>) {
    setBusy(label);
    setError(null);
    setMessage(null);
    try {
      await fn();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }

  if (items === null) {
    return <p className="px-4 py-4 text-xs text-neutral-400">読み込み中…</p>;
  }

  if (!current) {
    return (
      <div className="px-4 py-6 text-center">
        <p className="text-xs text-neutral-400">確認待ちの返信はありません。</p>
        {error && <p className="mt-2 text-xs break-words text-red-600">{error}</p>}
      </div>
    );
  }

  const generating = busy === "regen";

  return (
    <div className="flex h-full flex-col">
      <div className="flex shrink-0 items-center justify-between px-4 pt-3 pb-1">
        <span className="text-xs font-medium">
          {current.display_name}
          {items.length > 1 && (
            <span className="ml-2 text-neutral-400">
              {index + 1} / {items.length}
            </span>
          )}
        </span>
        <div className="flex gap-1">
          <button
            type="button"
            disabled={index === 0 || busy !== null}
            onClick={() => setIndex((i) => i - 1)}
            className="rounded border border-neutral-300 px-1.5 text-xs disabled:opacity-30 dark:border-neutral-600"
          >
            ←
          </button>
          <button
            type="button"
            disabled={index >= items.length - 1 || busy !== null}
            onClick={() => setIndex((i) => i + 1)}
            className="rounded border border-neutral-300 px-1.5 text-xs disabled:opacity-30 dark:border-neutral-600"
          >
            →
          </button>
        </div>
      </div>

      {/* スクロールするのはここだけ。 */}
      <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-2">
        <div className="rounded bg-neutral-100 p-2 text-xs whitespace-pre-wrap dark:bg-neutral-800">
          {current.incoming}
        </div>
        <div className="mt-1 text-[11px] text-neutral-400">
          {new Date(current.received_at * 1000).toLocaleString("ja-JP")}
          {current.reason && ` ・ ${current.reason}`}
        </div>

      </div>

      {/* ここから下は常に見える。書きかけの返信が隠れないようにする。 */}
      <div className="shrink-0 border-t border-neutral-200 px-4 pt-2 pb-2 dark:border-neutral-700">
        <div className="relative">
          <textarea
            ref={editor}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            disabled={busy !== null}
            placeholder="返信案"
            onKeyDown={(e) => {
              if (e.key === "Enter" && e.metaKey && draft.trim()) {
                e.preventDefault();
                void run("send", async () => {
                  setMessage(await sendReply(current.chat_rowid, draft));
                  await load();
                });
              }
            }}
            rows={5}
            className="w-full resize-none rounded border border-neutral-300 p-2 text-xs leading-relaxed disabled:opacity-40 dark:border-neutral-600 dark:bg-neutral-800"
          />

          {/* 生成中は入力欄が固まったように見える。何が起きているかを重ねて出す。 */}
          {busy !== null && (
            <div className="absolute inset-0 flex items-center justify-center rounded bg-white/75 dark:bg-neutral-900/75">
              <div className="flex items-center gap-2 text-xs text-neutral-600 dark:text-neutral-300">
                <Spinner />
                <span>{BUSY_LABEL[busy]}</span>
              </div>
            </div>
          )}
        </div>

        <div className="mt-2 flex flex-wrap gap-1">
          {LENGTH_PRESETS.map((p) => (
            <button
              key={p.id}
              type="button"
              disabled={busy !== null}
              onClick={() =>
                run("regen", async () => {
                  setDraft(await regenerate(current.chat_rowid, instruction || null, p.id));
                })
              }
              className="rounded border border-neutral-300 px-2 py-0.5 text-[11px] disabled:opacity-40 dark:border-neutral-600"
            >
              {p.label}
            </button>
          ))}
        </div>

        <input
          type="text"
          value={instruction}
          onChange={(e) => setInstruction(e.target.value)}
          disabled={busy !== null}
          placeholder="AIへの指示（任意）"
          className="mt-2 w-full rounded border border-neutral-300 px-2 py-1 text-xs disabled:opacity-40 dark:border-neutral-600 dark:bg-neutral-800"
        />

        <div className="mt-2 flex items-center gap-2">
          <button
            type="button"
            disabled={busy !== null}
            onClick={() =>
              run("regen", async () => {
                setDraft(await regenerate(current.chat_rowid, instruction || null, null));
              })
            }
            className="rounded border border-neutral-300 px-2 py-1 text-xs disabled:opacity-40 dark:border-neutral-600"
          >
            再生成
          </button>
          <button
            type="button"
            disabled={busy !== null || !draft.trim()}
            onClick={() =>
              run("send", async () => {
                setMessage(await sendReply(current.chat_rowid, draft));
                await load();
              })
            }
            className="flex items-center gap-1 rounded bg-blue-600 px-3 py-1 text-xs text-white disabled:opacity-40"
          >
            {busy === "send" && <Spinner light />}
            送信 ⌘↵
          </button>
          <button
            type="button"
            disabled={busy !== null}
            onClick={() =>
              run("skip", async () => {
                await skipPending(current.chat_rowid);
                await load();
              })
            }
            className="ml-auto text-xs text-neutral-500 disabled:opacity-40 dark:text-neutral-400"
          >
            返さない
          </button>
        </div>

        {generating && (
          <p className="mt-1 text-[11px] text-neutral-400">
            数秒かかります。完了すると案が入れ替わります。
          </p>
        )}
        {message && <p className="mt-1 text-xs text-green-600">{message}</p>}
        {error && <p className="mt-1 text-xs break-words text-red-600">{error}</p>}
      </div>
    </div>
  );
}

function Spinner({ light = false }: { light?: boolean }) {
  return (
    <span
      aria-hidden
      className={
        "inline-block h-3 w-3 animate-spin rounded-full border-2 border-t-transparent " +
        (light ? "border-white" : "border-neutral-400")
      }
    />
  );
}
