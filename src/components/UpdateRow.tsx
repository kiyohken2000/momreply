import { useEffect, useState } from "react";
import { appVersion, checkUpdate, installUpdate, type UpdateInfo } from "../api";
import { useLang } from "../lang";

/**
 * 版の表示と更新。
 *
 * # 自動で入れない理由
 *
 * このアプリは放置して自動送信する。裏で勝手に版が変わると、
 * 何が返信を書いたのか分からなくなる。確認だけ自動で、適用は人が押す。
 *
 * 更新は署名を検証してから適用される。公開鍵は tauri.conf.json にあり、
 * 対応する秘密鍵で署名されたものしか入らない。
 */
export default function UpdateRow() {
  const { t } = useLang();
  const [version, setVersion] = useState<string | null>(null);
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [busy, setBusy] = useState<null | "checking" | "installing">(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void appVersion().then(setVersion).catch(() => {});
    // 起動のたびに 1 回だけ静かに見る。見つかっても勝手には入れない。
    void checkUpdate()
      .then(setInfo)
      .catch(() => {
        // 電波が無いだけのこともある。ここでは黙る。
      });
  }, []);

  async function check() {
    setBusy("checking");
    setError(null);
    try {
      setInfo(await checkUpdate());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="border-b border-neutral-200 px-4 py-3 dark:border-neutral-700">
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs">
          {t("settings.version")} {version ?? "—"}
        </span>
        <button
          type="button"
          disabled={busy !== null}
          onClick={() => void check()}
          className="rounded border border-neutral-300 px-2 py-0.5 text-[11px] disabled:opacity-40 dark:border-neutral-600"
        >
          {busy === "checking" ? t("settings.checking") : t("settings.checkUpdate")}
        </button>
      </div>

      {info && !info.available && (
        <p className="mt-1 text-[11px] text-neutral-400">{t("settings.upToDate")}</p>
      )}

      {info?.available && (
        <div className="mt-2 rounded border border-blue-300 bg-blue-50 p-2 dark:border-blue-900 dark:bg-blue-950/40">
          <p className="text-[11px]">
            {t("settings.updateFound", { version: info.version })}
          </p>
          {info.notes && (
            <p className="mt-1 text-[10px] whitespace-pre-wrap text-neutral-500 dark:text-neutral-400">
              {info.notes}
            </p>
          )}
          <button
            type="button"
            disabled={busy !== null}
            onClick={() =>
              void (async () => {
                setBusy("installing");
                setError(null);
                try {
                  // 成功するとここで再起動する。戻ってこない。
                  await installUpdate();
                } catch (e) {
                  setError(String(e));
                  setBusy(null);
                }
              })()
            }
            className="mt-2 rounded bg-blue-600 px-3 py-1 text-[11px] text-white disabled:opacity-40"
          >
            {busy === "installing" ? t("settings.installing") : t("settings.install")}
          </button>
        </div>
      )}

      <p className="mt-1 text-[10px] text-neutral-400">{t("settings.updateNote")}</p>
      {error && <p className="mt-1 text-[11px] break-words text-red-600">{error}</p>}
    </div>
  );
}
