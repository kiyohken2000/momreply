import { useState } from "react";
import { openFullDiskAccessSettings, type ChatDbStatus } from "../api";

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
  const [opening, setOpening] = useState(false);

  if (!status.needs_full_disk_access) {
    return (
      <div className="h-full overflow-y-auto px-4 py-6">
        <h2 className="text-sm font-semibold">メッセージを読めません</h2>
        <p className="mt-2 text-xs text-neutral-500 dark:text-neutral-400">
          {status.reason}
        </p>
        <p className="mt-2 text-[11px] break-all text-neutral-400">{status.path}</p>
        <button
          type="button"
          onClick={onRecheck}
          className="mt-4 rounded border border-neutral-300 px-3 py-1 text-xs dark:border-neutral-600"
        >
          もう一度確認する
        </button>
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto px-4 py-6">
      <h2 className="text-sm font-semibold">フルディスクアクセスが必要です</h2>
      <p className="mt-2 text-xs leading-relaxed text-neutral-600 dark:text-neutral-300">
        MomReply は macOS のメッセージ履歴を<strong>読み取り専用で</strong>参照します。
        書き込みは一切しません。この許可が無いと、届いたメッセージを 1 件も
        読めません。
      </p>

      <ol className="mt-3 list-decimal space-y-1 pl-5 text-xs text-neutral-600 dark:text-neutral-300">
        <li>下のボタンでシステム設定を開く</li>
        <li>一覧に MomReply を追加して、スイッチを入れる</li>
        <li>
          <strong>MomReply を終了して、開き直す</strong>
        </li>
      </ol>

      <p className="mt-2 text-[11px] text-amber-600">
        許可したあとは必ず再起動してください。macOS は起動中のアプリに
        権限を反映しません。
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
          システム設定を開く
        </button>
        <button
          type="button"
          onClick={onRecheck}
          className="rounded border border-neutral-300 px-3 py-1 text-xs dark:border-neutral-600"
        >
          もう一度確認する
        </button>
      </div>

      <p className="mt-4 text-[11px] break-all text-neutral-400">
        読み取り先: {status.path}
      </p>
    </div>
  );
}
