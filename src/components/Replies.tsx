import { useCallback, useEffect, useRef, useState } from "react";
import {
  conversation,
  draftLatest,
  listPending,
  listTargets,
  recentConversation,
  regenerate,
  sendReply,
  skipPending,
  LENGTH_PRESETS,
  type Pending,
  type TargetView,
  type Turn,
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

  // 確認待ちが無いときこそ、返信を作りたい。相手タブまで探しに行かせない。
  if (!current) {
    return <Empty onDrafted={load} />;
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
        <Context chatRowid={current.chat_rowid} />

        {/* 返信の対象。連投なら、まとめた分がすべてここに入る。 */}
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

/**
 * 確認待ちが 1 件も無いときの画面。
 *
 * # なぜ会話を出すか
 *
 * 自動送信が有効だと、**うまく回っているほどここは空になる**。
 * 「確認待ちの返信はありません」だけだと、動いているのか止まって
 * いるのか区別がつかない。直近のやり取りを出せば、自動で送った返信も
 * そこに現れるので、状態がひと目で分かる。
 *
 * ここに「返信を作る」も置く。相手が最後に送ってきたまま止まっていても、
 * 監視は登録より前のメッセージを拾わない（仕様書 6.1 のバックログ保護）。
 * その状態で相手タブを探させるのは遠すぎる。
 *
 * 作った案は**必ず確認待ちに入る**。押しただけでは送信されない。
 */
function Empty({ onDrafted }: { onDrafted: () => Promise<void> }) {
  const [targets, setTargets] = useState<TargetView[] | null>(null);
  const [drafting, setDrafting] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    listTargets()
      .then(setTargets)
      .catch((e) => {
        setError(String(e));
        setTargets([]);
      });
  }, []);

  async function make(slug: string) {
    setDrafting(slug);
    setError(null);
    try {
      await draftLatest(slug);
      await onDrafted();
    } catch (e) {
      setError(String(e));
    } finally {
      setDrafting(null);
    }
  }

  return (
    <div className="h-full overflow-y-auto px-4 py-4">
      <p className="text-center text-xs text-neutral-400">確認待ちの返信はありません。</p>

      {targets !== null && targets.length === 0 && (
        <p className="mt-3 text-center text-xs text-neutral-400">
          相手タブで返信する相手を登録してください。
        </p>
      )}

      {targets?.map((t) => (
        <div key={t.slug} className="mt-4">
          <div className="text-[11px] font-medium text-neutral-500 dark:text-neutral-400">
            {t.display_name}
          </div>
          <Recent slug={t.slug} />
          <button
            type="button"
            disabled={drafting !== null}
            onClick={() => void make(t.slug)}
            className="mt-2 w-full rounded bg-blue-600 px-3 py-1.5 text-xs text-white disabled:opacity-40"
          >
            {drafting === t.slug ? "生成中…" : "直近の受信に返信を作る"}
          </button>
        </div>
      ))}

      {drafting !== null && (
        <p className="mt-2 text-center text-[11px] text-neutral-400">
          数十秒かかることがあります。
        </p>
      )}
      {error && <p className="mt-3 text-xs break-words text-red-600">{error}</p>}
    </div>
  );
}

/** 相手との直近のやり取り。自動で送った返信もここに出る。 */
function Recent({ slug }: { slug: string }) {
  const [turns, setTurns] = useState<Turn[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    recentConversation(slug)
      .then(setTurns)
      .catch((e) => {
        setError(String(e));
        setTurns([]);
      });
  }, [slug]);

  if (error) {
    return <p className="mt-1 text-[11px] break-words text-red-600">{error}</p>;
  }
  if (turns === null) {
    return <p className="mt-1 text-[11px] text-neutral-400">読み込み中…</p>;
  }
  if (turns.length === 0) {
    return <p className="mt-1 text-[11px] text-neutral-400">やり取りがありません。</p>;
  }

  return (
    <div className="mt-1 rounded border border-neutral-200 p-2 dark:border-neutral-700">
      {turns.map((t, i) => (
        <div key={i} className="mb-1.5 last:mb-0">
          <div className="text-[10px] text-neutral-400">
            {t.from_me ? "自分" : "相手"}
            {" ・ "}
            {new Date(t.at * 1000).toLocaleString("ja-JP", {
              month: "numeric",
              day: "numeric",
              hour: "2-digit",
              minute: "2-digit",
            })}
          </div>
          <div
            className={
              "text-[11px] whitespace-pre-wrap " +
              (t.from_me ? "text-blue-700 dark:text-blue-300" : "")
            }
          >
            {t.body}
          </div>
        </div>
      ))}
    </div>
  );
}

/**
 * 生成が読んだ、返信対象より前の会話。
 *
 * # なぜ既定で畳むか
 *
 * 20 件あると、肝心の「いま返信する相手のメッセージ」が上へ押し出される。
 * ポップオーバーは 380x560 しかない。ふだんは対象と下書きだけが見えていて、
 * 「なぜこの返信になったのか」を追いたいときだけ開ければいい。
 *
 * 開いたときに初めて読む。閉じたままなら chat.db を触らない。
 */
function Context({ chatRowid }: { chatRowid: number }) {
  const [open, setOpen] = useState(false);
  const [turns, setTurns] = useState<Turn[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  // 別の案に切り替わったら畳み直す。前の会話が残っていると読み違える。
  useEffect(() => {
    setOpen(false);
    setTurns(null);
    setError(null);
  }, [chatRowid]);

  useEffect(() => {
    if (!open || turns !== null) return;
    conversation(chatRowid)
      .then(setTurns)
      .catch((e) => {
        setError(String(e));
        setTurns([]);
      });
  }, [open, turns, chatRowid]);

  return (
    <div className="mb-2">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="text-[11px] text-neutral-500 hover:text-neutral-800 dark:text-neutral-400 dark:hover:text-neutral-100"
      >
        {open ? "▾ " : "▸ "}
        生成が読んだ会話
        {turns !== null && turns.length > 0 && `（${turns.length}件）`}
      </button>

      {open && (
        <div className="mt-1 rounded border border-neutral-200 p-2 dark:border-neutral-700">
          {turns === null && <p className="text-[11px] text-neutral-400">読み込み中…</p>}
          {turns?.length === 0 && !error && (
            <p className="text-[11px] text-neutral-400">この前のやり取りはありません。</p>
          )}
          {turns?.map((t, i) => (
            <div key={i} className="mb-1 flex gap-1.5 text-[11px] last:mb-0">
              <span
                className={
                  "shrink-0 " + (t.from_me ? "text-blue-600" : "text-neutral-400")
                }
              >
                {t.from_me ? "自分" : "相手"}
              </span>
              <span className="whitespace-pre-wrap">{t.body}</span>
            </div>
          ))}
          {error && <p className="text-[11px] break-words text-red-600">{error}</p>}
          <p className="mt-2 text-[10px] text-neutral-400">
            返信案を作るときに、これがそのまま LLM へ渡っています。
          </p>
        </div>
      )}
    </div>
  );
}
