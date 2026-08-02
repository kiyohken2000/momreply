import { useState } from "react";
import { openFullDiskAccessSettings, type ChatDbStatus } from "../api";
import { useLang } from "../lang";

/**
 * chat.db が読めないときの案内。
 *
 * フルディスクアクセスが無いと、ファイルは**存在するのに開けない**。
 * この状態で普通の画面を出すと、相手も登録できず返信も来ないまま、
 * 何が悪いのか分からない時間が続く。だから他の画面より前に出す。
 *
 * 権限を与えたあとは**アプリの再起動が要る**。macOS は起動中の
 * プロセスに権限を反映しない。ここを書いておかないと、
 * 「許可したのに直らない」で止まる。
 */
export default function NeedsAccess({
  status,
  onRecheck,
}: {
  status: ChatDbStatus;
  onRecheck: () => void;
}) {
  const { t } = useLang();
  const [opening, setOpening] = useState(false);

  if (!status.needs_full_disk_access) {
    return (
      <div className="h-full overflow-y-auto px-4 py-6">
        <h2 className="text-sm font-semibold">{t("access.cannotRead")}</h2>
        <p className="mt-2 text-xs text-neutral-500 dark:text-neutral-400">
          {status.reason}
        </p>
        <p className="mt-2 text-[11px] break-all text-neutral-400">
          {status.path}
        </p>
        <button
          type="button"
          onClick={onRecheck}
          className="mt-4 rounded border border-neutral-300 px-3 py-1 text-xs dark:border-neutral-600"
        >
          {t("common.recheck")}
        </button>
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto px-4 py-6">
      <h2 className="text-sm font-semibold">{t("access.title")}</h2>
      <p className="mt-2 text-xs leading-relaxed text-neutral-600 dark:text-neutral-300">
        {t("access.body")}
      </p>

      <ol className="mt-3 list-decimal space-y-1 pl-5 text-xs text-neutral-600 dark:text-neutral-300">
        <li>{t("access.step1")}</li>
        <li>{t("access.step2")}</li>
        <li>
          <strong>{t("access.step3")}</strong>
        </li>
      </ol>

      <p className="mt-2 text-[11px] text-amber-600">
        {t("access.restartWarning")}
      </p>

      <div className="mt-4 flex flex-wrap gap-2">
        <button
          type="button"
          disabled={opening}
          onClick={() =>
            void (async () => {
              setOpening(true);
              try {
                await openFullDiskAccessSettings();
              } finally {
                setOpening(false);
              }
            })()
          }
          className="rounded bg-blue-600 px-3 py-1 text-xs text-white disabled:opacity-40"
        >
          {t("access.openSettings")}
        </button>
        <button
          type="button"
          onClick={onRecheck}
          className="rounded border border-neutral-300 px-3 py-1 text-xs dark:border-neutral-600"
        >
          {t("common.recheck")}
        </button>
      </div>

      <p className="mt-4 text-[11px] break-all text-neutral-400">
        {t("access.readFrom")}: {status.path}
      </p>
    </div>
  );
}
