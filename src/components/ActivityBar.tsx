import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { currentActivity, type Activity } from "../api";

/** 状態が変わったことを知らせる合図。Rust 側の `EVENT_ACTIVITY` と対。 */
const ACTIVITY = "momreply://activity";

/**
 * 裏で動いていることを示す帯。
 *
 * 自動送信は 1 往復に 1 分半ほどかかる（連投待ち 45 秒 + 生成 50 秒）。
 * その間ポップオーバーを開いても何も出ないと、止まっているのか
 * 動いているのか分からない。
 *
 * どのタブを見ていても目に入るよう、ヘッダーに置く。
 */
export default function ActivityBar() {
  const [activity, setActivity] = useState<Activity | null>(null);

  useEffect(() => {
    // 開いた時点で既に動いていることがある。合図では拾えないので読みに行く。
    void currentActivity().then(setActivity).catch(() => {});

    const un = listen<Activity | null>(ACTIVITY, (e) => setActivity(e.payload));
    return () => {
      void un.then((f) => f());
    };
  }, []);

  if (!activity) return null;

  return (
    <div className="flex items-center gap-2 border-b border-blue-200 bg-blue-50 px-4 py-1.5 dark:border-blue-900 dark:bg-blue-950/40">
      <span
        aria-hidden
        className="inline-block h-3 w-3 shrink-0 animate-spin rounded-full border-2 border-blue-500 border-t-transparent"
      />
      <span className="text-[11px] text-blue-800 dark:text-blue-200">
        {activity.who} ・ {activity.label}
      </span>
    </div>
  );
}
