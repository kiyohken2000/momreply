import { useCallback, useEffect, useRef, useState } from "react";
import {
  answerQuestion,
  listPending,
  regenerate,
  sendReply,
  skipPending,
  LENGTH_PRESETS,
  type Pending,
} from "../api";

/**
 * 確認待ちの返信（仕様書 6.6）。
 *
 * 主動線は「返信案をその場で直して送る」なので、テキストエリアに
 * 最初からフォーカスを当てる。⌘Enter で送信。
 */
export default function Replies() {
  const [items, setItems] = useState<Pending[] | null>(null);
  const [index, setIndex] = useState(0);
  const [draft, setDraft] = useState("");
  const [instruction, setInstruction] = useState("");
  const [busy, setBusy] = useState<null | string>(null);
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

  // 表示中の 1 件が変わったら、その返信案を編集欄に入れ直す。
  useEffect(() => {
    setDraft(current?.draft ?? "");
    setInstruction("");
    setMessage(null);
  }, [current?.chat_rowid, current?.draft]);

  useEffect(() => {
    if (current && draft) editor.current?.focus();
  }, [current?.chat_rowid]);

  async function run(label: string, fn: () => Promise<void>) {
    setBusy(label);
    setError(null);
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

  const needsAnswer = current.questions.length > 0;

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
            disabled={index === 0}
            onClick={() => setIndex((i) => i - 1)}
            className="rounded border border-neutral-300 px-1.5 text-xs disabled:opacity-30 dark:border-neutral-600"
          >
            ←
          </button>
          <button
            type="button"
            disabled={index >= items.length - 1}
            onClick={() => setIndex((i) => i + 1)}
            className="rounded border border-neutral-300 px-1.5 text-xs disabled:opacity-30 dark:border-neutral-600"
          >
            →
          </button>
        </div>
      </div>

      <div className="shrink-0 px-4 pb-2">
        <div className="max-h-24 overflow-y-auto rounded bg-neutral-100 p-2 text-xs whitespace-pre-wrap dark:bg-neutral-800">
          {current.incoming}
        </div>
        <div className="mt-1 text-[11px] text-neutral-400">
          {new Date(current.received_at * 1000).toLocaleString("ja-JP")}
          {current.reason && ` ・ ${current.reason}`}
        </div>
      </div>

      {/* 材料不足で止まった場合は、まずここを埋めないと直らない。 */}
      {needsAnswer && (
        <div className="mx-4 mb-2 shrink-0 rounded border border-amber-300 bg-amber-50 p-2 dark:border-amber-700 dark:bg-amber-950/40">
          <p className="mb-1 text-xs font-medium">答える材料がありません</p>
          {current.questions.map((q) => (
            <QuestionAnswer
              key={q.id}
              question={q.question}
              disabled={busy !== null}
              onAnswer={(answer) =>
                run("answer", async () => {
                  await answerQuestion(q.id, answer);
                  await regenerate(current.chat_rowid, null, null);
                  await load();
                })
              }
            />
          ))}
        </div>
      )}

      <div className="flex min-h-0 flex-1 flex-col px-4 pb-3">
        <textarea
          ref={editor}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          disabled={busy !== null}
          placeholder={needsAnswer ? "上の質問に答えると案が作られます" : "返信案"}
          onKeyDown={(e) => {
            // ⌘Enter で送信（仕様書 6.6）。
            if (e.key === "Enter" && e.metaKey && draft.trim()) {
              e.preventDefault();
              void run("send", async () => {
                setMessage(await sendReply(current.chat_rowid, draft));
                await load();
              });
            }
          }}
          className="min-h-20 w-full flex-1 resize-none rounded border border-neutral-300 p-2 text-xs leading-relaxed disabled:opacity-50 dark:border-neutral-600 dark:bg-neutral-800"
        />

        <div className="mt-2 flex shrink-0 flex-wrap gap-1">
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
          className="mt-2 w-full shrink-0 rounded border border-neutral-300 px-2 py-1 text-xs disabled:opacity-50 dark:border-neutral-600 dark:bg-neutral-800"
        />

        <div className="mt-2 flex shrink-0 items-center gap-2">
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
            {busy === "regen" ? "生成中…" : "再生成"}
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
            className="rounded bg-blue-600 px-3 py-1 text-xs text-white disabled:opacity-40"
          >
            {busy === "send" ? "送信中…" : "送信 ⌘↵"}
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

        {message && <p className="mt-2 shrink-0 text-xs text-green-600">{message}</p>}
        {error && <p className="mt-2 shrink-0 text-xs break-words text-red-600">{error}</p>}
      </div>
    </div>
  );
}

function QuestionAnswer({
  question,
  disabled,
  onAnswer,
}: {
  question: string;
  disabled: boolean;
  onAnswer: (answer: string) => void;
}) {
  const [answer, setAnswer] = useState("");
  return (
    <form
      className="mb-1 last:mb-0"
      onSubmit={(e) => {
        e.preventDefault();
        if (answer.trim()) onAnswer(answer.trim());
      }}
    >
      <div className="text-[11px]">{question}</div>
      <div className="mt-0.5 flex gap-1">
        <input
          type="text"
          value={answer}
          onChange={(e) => setAnswer(e.target.value)}
          disabled={disabled}
          placeholder="答え"
          className="min-w-0 flex-1 rounded border border-neutral-300 px-2 py-0.5 text-[11px] disabled:opacity-50 dark:border-neutral-600 dark:bg-neutral-800"
        />
        <button
          type="submit"
          disabled={disabled || !answer.trim()}
          className="rounded bg-blue-600 px-2 py-0.5 text-[11px] text-white disabled:opacity-40"
        >
          登録
        </button>
      </div>
    </form>
  );
}
