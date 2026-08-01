import { useCallback, useEffect, useState } from "react";
import {
  approveFact,
  getSelfProfile,
  listFactCandidates,
  rejectFact,
  setSelfProfile,
  type FactCandidate,
} from "../api";

/**
 * `self.md` の編集と、追記候補の承認。
 *
 * 候補は**承認するまで反映しない**。self.md は AI が事実として断定する
 * 唯一の材料なので、誤りが 1 行入ると以後すべての生成が汚染される。
 * だから根拠のやり取りを必ず並べて、人が判断できるようにしている。
 */
export default function SelfProfile() {
  const [text, setText] = useState<string | null>(null);
  const [saved, setSaved] = useState<string>("");
  const [candidates, setCandidates] = useState<FactCandidate[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [content, facts] = await Promise.all([getSelfProfile(), listFactCandidates()]);
      setText(content);
      setSaved(content);
      setCandidates(facts);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const dirty = text !== null && text !== saved;

  async function save() {
    if (text === null) return;
    setBusy(true);
    try {
      await setSelfProfile(text);
      setSaved(text);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function decide(id: number, approve: boolean) {
    setBusy(true);
    try {
      if (approve) {
        // 追記後の全文が返る。編集中の内容と食い違わないよう置き換える。
        const updated = await approveFact(id);
        setText(updated);
        setSaved(updated);
      } else {
        await rejectFact(id);
      }
      setCandidates((prev) => prev.filter((c) => c.id !== id));
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section>
      <h2 className="px-4 pt-4 pb-2 text-xs font-semibold tracking-wide text-neutral-500 uppercase dark:text-neutral-400">
        自分について
      </h2>
      <p className="px-4 pb-2 text-xs text-neutral-500 dark:text-neutral-400">
        ここに書いた内容だけを、AI は事実として断定します。書かれていないことは
        推測せず、あなたに確認を求めます。
      </p>

      {error && <p className="px-4 pb-2 text-xs break-words text-red-600">{error}</p>}

      {candidates.length > 0 && (
        <div className="mx-4 mb-3 rounded border border-amber-300 bg-amber-50 p-2 dark:border-amber-700 dark:bg-amber-950/40">
          <p className="mb-2 text-xs font-medium">
            追記候補 {candidates.length} 件（承認するまで反映されません）
          </p>
          {candidates.map((c) => (
            <div
              key={c.id}
              className="mb-2 border-b border-amber-200 pb-2 last:mb-0 last:border-b-0 last:pb-0 dark:border-amber-800"
            >
              <div className="text-xs font-medium">{c.content}</div>
              {c.evidence_ask && (
                <div className="mt-1 text-[11px] leading-snug text-neutral-500 dark:text-neutral-400">
                  <div className="line-clamp-2">相手:「{c.evidence_ask}」</div>
                  <div>自分:「{c.evidence_reply}」</div>
                </div>
              )}
              <div className="mt-1 flex gap-2">
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => void decide(c.id, true)}
                  className="rounded bg-blue-600 px-2 py-0.5 text-[11px] text-white disabled:opacity-40"
                >
                  承認
                </button>
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => void decide(c.id, false)}
                  className="rounded border border-neutral-300 px-2 py-0.5 text-[11px] disabled:opacity-40 dark:border-neutral-600"
                >
                  却下
                </button>
              </div>
            </div>
          ))}
        </div>
      )}

      <div className="px-4 pb-4">
        <textarea
          value={text ?? ""}
          onChange={(e) => setText(e.target.value)}
          disabled={text === null || busy}
          spellCheck={false}
          rows={12}
          className="w-full rounded border border-neutral-300 p-2 font-mono text-xs leading-relaxed disabled:opacity-50 dark:border-neutral-600 dark:bg-neutral-800"
        />
        <div className="mt-2 flex items-center gap-2">
          <button
            type="button"
            onClick={() => void save()}
            disabled={!dirty || busy}
            className="rounded bg-blue-600 px-3 py-1 text-xs text-white disabled:opacity-40"
          >
            保存
          </button>
          {dirty && <span className="text-xs text-amber-600">未保存の変更があります</span>}
        </div>
        <p className="mt-2 text-[11px] text-neutral-400">
          この内容は返信生成のたびに LLM へ送られます。外部に出て困ることは
          書かないでください。
        </p>
      </div>
    </section>
  );
}
