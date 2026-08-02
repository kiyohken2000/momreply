import { useCallback, useEffect, useState } from "react";
import {
  approveFact,
  getSelfProfile,
  listFactCandidates,
  rejectFact,
  setSelfProfile,
  type FactCandidate,
} from "../api";
import { useLang } from "../lang";

/**
 * `self.md` の編集と、追記候補の承認。
 *
 * self.md は 2 つの役割を持つ。**文章の方向性の指示**（「デスマス調に
 * しない」など。文例より優先される）と、**言い切ってよい事実**である。
 *
 * 候補は**承認するまで反映しない**。誤りが 1 行入ると以後すべての生成が
 * 汚染されるため、根拠のやり取りを必ず並べて人が判断できるようにしている。
 */
export default function SelfProfile() {
  const { t } = useLang();
  const [text, setText] = useState<string | null>(null);
  const [saved, setSaved] = useState<string>("");
  const [candidates, setCandidates] = useState<FactCandidate[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [candidateError, setCandidateError] = useState<string | null>(null);

  // 2 つは独立して読む。まとめて待つと、候補の取得に失敗しただけで
  // 本文が null のままになり、編集も保存もできなくなる。
  const load = useCallback(async () => {
    try {
      const content = await getSelfProfile();
      setText(content);
      setSaved(content);
      setError(null);
    } catch (e) {
      setError(t("self.readError", { reason: String(e) }));
      // 読めなくても編集はできるようにしておく。
      setText("");
      setSaved("");
    }

    try {
      setCandidates(await listFactCandidates());
      setCandidateError(null);
    } catch (e) {
      setCandidateError(String(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const loading = text === null;
  const dirty = !loading && text !== saved;

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
    <section className="flex h-full flex-col">
      <h2 className="shrink-0 px-4 pt-4 pb-2 text-xs font-semibold tracking-wide text-neutral-500 uppercase dark:text-neutral-400">
        {t("self.title")}
      </h2>
      <p className="shrink-0 px-4 pb-2 text-xs text-neutral-500 dark:text-neutral-400">
        {t("self.lead")}
      </p>

      {error && (
        <p className="px-4 pb-2 text-xs break-words text-red-600">{error}</p>
      )}
      {candidateError && (
        <p className="px-4 pb-2 text-xs break-words text-amber-600">
          {t("self.candidateError", { reason: candidateError })}
        </p>
      )}

      {/* 候補が多いときはここだけスクロールさせ、
          本文の編集領域を潰さない。 */}
      {candidates.length > 0 && (
        <div className="mx-4 mb-3 max-h-56 shrink-0 overflow-y-auto rounded border border-amber-300 bg-amber-50 p-2 dark:border-amber-700 dark:bg-amber-950/40">
          <p className="mb-2 text-xs font-medium">
            {t("self.candidates", { n: candidates.length })}
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
                  {t("self.approve")}
                </button>
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => void decide(c.id, false)}
                  className="rounded border border-neutral-300 px-2 py-0.5 text-[11px] disabled:opacity-40 dark:border-neutral-600"
                >
                  {t("self.reject")}
                </button>
              </div>
            </div>
          ))}
        </div>
      )}

      <div className="flex min-h-0 flex-1 flex-col px-4 pb-4">
        {/* 行数を固定せず、残りの高さいっぱいに広げる。
            ウィンドウの大きさが変わっても余白が出ない。 */}
        <textarea
          value={text ?? ""}
          onChange={(e) => setText(e.target.value)}
          disabled={loading || busy}
          placeholder={loading ? t("common.loading") : t("self.placeholder")}
          spellCheck={false}
          className="min-h-32 w-full flex-1 resize-none rounded border border-neutral-300 p-2 font-mono text-xs leading-relaxed disabled:opacity-50 dark:border-neutral-600 dark:bg-neutral-800"
        />
        <div className="mt-2 flex shrink-0 items-center gap-2">
          {/* 変更が無くても押せるようにする。押せない理由が画面から
              分からないと、壊れているのか仕様なのか区別できない。 */}
          <button
            type="button"
            onClick={() => void save()}
            disabled={loading || busy}
            className="rounded bg-blue-600 px-3 py-1 text-xs text-white disabled:opacity-40"
          >
            {busy ? t("common.saving") : t("common.save")}
          </button>
          {dirty ? (
            <span className="text-xs text-amber-600">
              {t("common.unsaved")}
            </span>
          ) : (
            !loading && (
              <span className="text-xs text-neutral-400">
                {t("common.saved")}
              </span>
            )
          )}
        </div>
        <p className="mt-2 shrink-0 text-[11px] text-neutral-400">
          {t("self.sentToLlm")}
        </p>
      </div>
    </section>
  );
}
