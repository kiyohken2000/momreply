//! 新着メッセージの検知（仕様書 6.1）。
//!
//! # トリガー
//!
//! 1. 定期ポーリング（既定 5 秒）
//! 2. `chat.db-wal` の変更を FSEvents で監視して即座にポーリング
//! 3. 時刻ギャップの検出
//!
//! 仕様書は 3 番目を `NSWorkspace.didWakeNotification` の購読としているが、
//! ここでは**前回ポーリングからの経過時間**で判定している。スリープ復帰を
//! 直接購読しなくても、防ぎたい事故（溜まったメッセージへの一斉返信）は
//! 同じように防げる。加えて、アプリが止まっていた場合や時計が飛んだ場合も
//! 拾える。復帰通知の購読は Objective-C の橋渡しが要るため、必要になった
//! 時点で足す。
//!
//! # 選び方（重要）
//!
//! 新着が複数あっても**返信するのは 1 回だけ**にする。
//! 行ごとに返すと会話が壊れる。
//!
//! ただし相手は 1 つの用件を数行に分けて送ってくる。最後の 1 行だけを
//! 見ると、中身がそこに無いことがある（「返信なければ行く」だけが残り、
//! 実際の問いは前の行にある）。そこで、自分の返信を挟まずに短時間で
//! 続いた相手のメッセージは、**1 通としてまとめて**生成に渡す
//! （[`burst`]）。まとめた分は `merged`、追い越された分は `superseded`、
//! 時刻ギャップ後なら `stale` として記録に残す。

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};
use rusqlite::Connection;

use super::Message;

/// 連投とみなす間隔。これより長く空いたら別の話として切る。
pub const BURST_WINDOW: Duration = Duration::from_secs(5 * 60);

/// 1 通にまとめる上限。
///
/// 際限なくまとめると、長い独白がまるごとプロンプトに入って
/// 生成が壊れる。塊としての意味が残る範囲で切る。
pub const BURST_MAX: usize = 10;

/// 連投を探すために遡る件数。
const BURST_LOOKBACK: u32 = 30;

/// 見送った理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Passed {
    /// 同じ相手からより新しいメッセージが来ている。
    Superseded,
    /// 連投の一部として、生成に取り込まれた。**無視されていない。**
    Merged,
    /// 時刻が飛んだあとに溜まっていた分（仕様書 6.1）。
    Stale,
    /// 自分の送信・タップバックなど、そもそも対象外。
    NotApplicable,
}

impl Passed {
    pub fn label(self) -> &'static str {
        match self {
            Passed::Superseded => "superseded",
            Passed::Merged => "merged",
            Passed::Stale => "stale",
            Passed::NotApplicable => "not_applicable",
        }
    }
}

/// 何を処理し、何を見送るか。
#[derive(Debug, Clone)]
pub struct Plan {
    /// 生成対象。無いこともある。
    pub actionable: Option<Message>,
    /// 見送った分。記録のために理由を添える。
    pub passed: Vec<(Message, Passed)>,
    /// 次回の起点。**処理対象が無くてもここまでは進める。**
    /// でないと同じメッセージを何度も拾い続ける。
    pub next_seen_rowid: Option<i64>,
}

/// 新着から処理対象を決める。
///
/// `gap_detected` が真なら、時刻が飛んだ後の一括処理として扱う。
pub fn plan(messages: Vec<Message>, gap_detected: bool) -> Plan {
    let next_seen_rowid = messages.iter().map(|m| m.rowid).max();

    let mut candidates = Vec::new();
    let mut passed = Vec::new();

    for m in messages {
        // 自分の送信は絶対にトリガーにしない（仕様書 6.4.4）。
        if m.is_from_me || m.skip.is_some() {
            passed.push((m, Passed::NotApplicable));
        } else {
            candidates.push(m);
        }
    }

    let actionable = candidates.pop();
    let reason = if gap_detected {
        Passed::Stale
    } else {
        Passed::Superseded
    };
    passed.extend(candidates.into_iter().map(|m| (m, reason)));

    Plan {
        actionable,
        passed,
        next_seen_rowid,
    }
}

/// [`plan`] の結果に、連投のまとめを反映する。
///
/// 連投の前半は「より新しいものに追い越された」のではなく、**生成に
/// 取り込まれる**。理由をそのままにすると、記録を見たときに
/// 「無視された」と読めてしまう。
pub fn plan_with_burst(
    conn: &Connection,
    handles: &[String],
    messages: Vec<Message>,
    gap_detected: bool,
) -> Result<Plan> {
    let mut plan = plan(messages, gap_detected);

    let Some(target) = &plan.actionable else {
        return Ok(plan);
    };
    let merged: Vec<i64> = burst(conn, handles, target, BURST_WINDOW)?
        .iter()
        .map(|m| m.rowid)
        .collect();

    for (m, reason) in &mut plan.passed {
        if *reason != Passed::NotApplicable && merged.contains(&m.rowid) {
            *reason = Passed::Merged;
        }
    }
    Ok(plan)
}

/// `target` を末尾とする連投を、古い順に返す。
///
/// 単発なら `target` 1 件だけが返る。
pub fn burst(
    conn: &Connection,
    handles: &[String],
    target: &Message,
    window: Duration,
) -> Result<Vec<Message>> {
    let recent = super::reader::recent_messages(conn, handles, BURST_LOOKBACK)?;
    Ok(group_burst(&recent, target.rowid, window))
}

/// 連投のまとめ方（純粋関数）。`recent` は古い順。
///
/// `rowid` から遡り、次のいずれかで切る。
///
/// - **自分の送信が挟まった** — そこから先は別の話。
/// - **間隔が `window` を超えた** — 続きではなく新しい用件。
/// - **`BURST_MAX` 件に達した**。
///
/// タップバックなどの対象外メッセージは、塊を割らずに読み飛ばす。
/// 「👍」が 1 つ挟まっただけで連投が分断されると、まとめる意味が無い。
pub fn group_burst(recent: &[Message], rowid: i64, window: Duration) -> Vec<Message> {
    let Some(end) = recent.iter().position(|m| m.rowid == rowid) else {
        return Vec::new();
    };

    let mut out = vec![recent[end].clone()];
    let mut previous = recent[end].date;

    for m in recent[..end].iter().rev() {
        if out.len() >= BURST_MAX {
            break;
        }
        // 自分が返していたら、そこで話は切れている。
        if m.is_from_me {
            break;
        }
        if m.skip.is_some() {
            continue;
        }
        match previous.signed_duration_since(m.date).to_std() {
            Ok(gap) if gap <= window => {}
            // 空きすぎ、または並びが壊れている。まとめない。
            _ => break,
        }
        previous = m.date;
        out.push(m.clone());
    }

    out.reverse();
    out
}

/// 連投を 1 通の本文としてつなぐ。
///
/// 本文の無いものは落とす。改行で繋ぐのは、相手が実際に
/// 改行入りの 1 通として送ってくる形と同じにするため。
pub fn burst_text(messages: &[Message]) -> String {
    messages
        .iter()
        .filter_map(|m| m.body.as_deref())
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 連投がそろうのを待つ時間。
///
/// 受信した瞬間に生成を始めると、生成中に届いた続きを取りこぼす。
/// 実際に起きた形はこうだった。
///
/// ```text
/// 13:26:52  相手  あなたの文章全くわからない   ← これで生成を開始
/// 13:27:04  相手  あなたは、体調が悪い？        ← 生成中に到着
/// 13:27:10  相手  山木戸？                     ← 生成中に到着
/// 13:27:49  自分  （1 通目への返信を送信）
/// 13:27:55        後の 2 件は already_replied で永久にスキップ
/// ```
///
/// 既返信ガードは正しく働いている。問題は**返信を書き始めるのが早すぎる**
/// ことのほうで、静かになるまで待てば [`burst`] が全部まとめられる。
pub const SETTLE_WINDOW: Duration = Duration::from_secs(45);

/// これ以上は待たない。
///
/// 相手が延々と送り続けている間、待ち続けると一度も返信できない。
pub const SETTLE_MAX_WAIT: Duration = Duration::from_secs(5 * 60);

/// まだ続きが来るかもしれないか。真なら今回は何もしない。
///
/// `newest` は返信対象、`oldest` は未処理のうち最も古い受信。
/// 新しいものが来るたびに待ち直すが、`max_wait` で頭打ちにする。
pub fn is_settling(
    newest: DateTime<Local>,
    oldest: DateTime<Local>,
    now: DateTime<Local>,
    window: Duration,
    max_wait: Duration,
) -> bool {
    let elapsed = |t: DateTime<Local>| now.signed_duration_since(t).to_std().ok();
    match (elapsed(newest), elapsed(oldest)) {
        (Some(newest), Some(oldest)) => newest < window && oldest < max_wait,
        // 受信時刻が未来。時計がずれている。
        // 経過 0 として扱うと、いつまでも「たった今届いた」ままになり
        // 一度も返信できなくなる。待たずに進める。
        _ => false,
    }
}

/// 受信からの経過が長いか（仕様書 6.4.2 stale guard）。
///
/// 真なら自動送信せず確認に回す。返信するには古すぎる話に、
/// 何時間も経ってから急に返すのを防ぐ。
pub fn is_stale(received: DateTime<Local>, now: DateTime<Local>, threshold: Duration) -> bool {
    match now.signed_duration_since(received).to_std() {
        Ok(elapsed) => elapsed > threshold,
        // 受信時刻が未来。時計のずれなので古くはない。
        Err(_) => false,
    }
}

/// 前回ポーリングからの経過が長いか（仕様書 6.1）。
pub fn gap_detected(last_poll: Option<i64>, now: i64, threshold: Duration) -> bool {
    match last_poll {
        Some(prev) => now.saturating_sub(prev) > threshold.as_secs() as i64,
        // 初回は判定しない。バックログ保護が別に効いている。
        None => false,
    }
}

/// 監視の設定。
#[derive(Debug, Clone)]
pub struct Config {
    pub poll_interval: Duration,
    pub wake_gap_threshold: Duration,
    /// `chat.db-wal` は書き込みのたびに何度も通知が来る。まとめる。
    pub debounce: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            poll_interval: Duration::from_secs(5),
            wake_gap_threshold: Duration::from_secs(600),
            debounce: Duration::from_millis(500),
        }
    }
}

/// ポーリングを起こす合図。
pub enum Tick {
    /// 定期ポーリング。
    Interval,
    /// `chat.db-wal` が変化した。
    FileChanged,
}

/// `chat.db-wal` を監視しつつ、一定間隔でも合図を送る。
///
/// 呼び出し側はこのイテレータを回して、合図が来るたびに新着を確認する。
pub struct Ticker {
    _watcher: Option<RecommendedWatcher>,
    rx: mpsc::Receiver<()>,
    config: Config,
}

impl Ticker {
    pub fn new(chat_db: &Path, config: Config) -> Result<Self> {
        let (tx, rx) = mpsc::channel();

        // -wal を直接指定すると、SQLite が作り直したときに監視が外れる。
        // ディレクトリを見て、chat.db 系のファイルだけを拾う。
        let dir = chat_db
            .parent()
            .context("chat.db の親ディレクトリが取れない")?
            .to_path_buf();
        let stem = chat_db
            .file_name()
            .context("chat.db のファイル名が取れない")?
            .to_os_string();

        let watcher = match RecommendedWatcher::new(
            move |res: notify::Result<notify::Event>| {
                let Ok(event) = res else { return };
                if event.paths.iter().any(|p| is_chat_db_file(p, &stem)) {
                    // 受け手が居なくなっていても監視側は落とさない。
                    let _ = tx.send(());
                }
            },
            notify::Config::default(),
        ) {
            Ok(mut w) => match w.watch(&dir, RecursiveMode::NonRecursive) {
                Ok(()) => Some(w),
                Err(why) => {
                    // 監視できなくても定期ポーリングだけで動作は続く。
                    eprintln!("警告: chat.db の監視を開始できない（ポーリングのみで継続）: {why}");
                    None
                }
            },
            Err(why) => {
                eprintln!("警告: ファイル監視を初期化できない（ポーリングのみで継続）: {why}");
                None
            }
        };

        Ok(Ticker {
            _watcher: watcher,
            rx,
            config,
        })
    }

    /// 次の合図まで待つ。
    pub fn wait(&self) -> Tick {
        match self.rx.recv_timeout(self.config.poll_interval) {
            Ok(()) => {
                // まとめて来る通知を吸収する。
                std::thread::sleep(self.config.debounce);
                while self.rx.try_recv().is_ok() {}
                Tick::FileChanged
            }
            Err(_) => Tick::Interval,
        }
    }
}

fn is_chat_db_file(path: &Path, stem: &std::ffi::OsStr) -> bool {
    path.file_name()
        .map(|name| {
            let name = name.to_string_lossy();
            let stem = stem.to_string_lossy();
            name == stem || name.starts_with(&format!("{stem}-"))
        })
        .unwrap_or(false)
}

/// 監視対象の既定パス群（デバッグ表示用）。
pub fn watched_files(chat_db: &Path) -> Vec<PathBuf> {
    let mut out = vec![chat_db.to_path_buf()];
    for suffix in ["-wal", "-shm"] {
        let mut p = chat_db.as_os_str().to_os_string();
        p.push(suffix);
        out.push(PathBuf::from(p));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn msg(rowid: i64, from_me: bool) -> Message {
        Message {
            rowid,
            guid: format!("g{rowid}"),
            chat_identifier: "x@example.com".into(),
            date: Local::now(),
            is_from_me: from_me,
            edited: false,
            body: Some(format!("本文{rowid}")),
            skip: None,
            body_from_text_column: false,
        }
    }

    // MARK: 連投がそろうのを待つ

    const SETTLE: Duration = Duration::from_secs(45);
    const MAX_WAIT: Duration = Duration::from_secs(300);

    fn t(secs: i64) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 8, 2, 13, 26, 52).unwrap() + chrono::Duration::seconds(secs)
    }

    /// 届いた直後に書き始めると、生成中に来た続きを取りこぼす。
    #[test]
    fn a_fresh_message_waits_for_the_rest() {
        assert!(is_settling(t(0), t(0), t(10), SETTLE, MAX_WAIT));
    }

    #[test]
    fn after_the_window_it_proceeds() {
        assert!(!is_settling(t(0), t(0), t(46), SETTLE, MAX_WAIT));
    }

    /// 続きが来たら待ち直す。18 秒後の 3 通目で、待ちは仕切り直しになる。
    #[test]
    fn a_new_message_restarts_the_wait() {
        // 1 通目から 40 秒。本来ならもうすぐ書き始めるところ。
        // しかし 18 秒前に 3 通目が来ているので、まだ待つ。
        assert!(is_settling(t(18), t(0), t(40), SETTLE, MAX_WAIT));
    }

    /// 送り続けられている間、待ち続けると一度も返信できない。
    #[test]
    fn the_wait_is_capped() {
        // 直前にも届いているが、最初の 1 通から 5 分を超えた。
        assert!(!is_settling(t(295), t(0), t(301), SETTLE, MAX_WAIT));
    }

    /// 時計が飛んで受信時刻が未来になっても、待ち続けない。
    #[test]
    fn a_future_timestamp_does_not_hang_the_wait() {
        assert!(!is_settling(t(100), t(100), t(0), SETTLE, MAX_WAIT));
    }

    /// 連投の検証用。`at` は基準時刻からの経過秒。
    fn at(rowid: i64, from_me: bool, secs: i64, body: &str) -> Message {
        let base = Local.with_ymd_and_hms(2026, 8, 1, 9, 0, 0).unwrap();
        Message {
            date: base + chrono::Duration::seconds(secs),
            body: Some(body.into()),
            ..msg(rowid, from_me)
        }
    }

    const WINDOW: Duration = Duration::from_secs(300);

    /// 実データにあった形。最後の 1 行だけでは中身が無い。
    #[test]
    fn consecutive_incoming_messages_become_one() {
        let recent = vec![
            at(1, true, 0, "来ない"),
            at(2, false, 60, "なら、答えて下さい"),
            at(3, false, 70, "マイナンバーカードについても明確な返信下さい"),
            at(4, false, 80, "資格確認証は、ありますか？"),
            at(5, false, 90, "返信なければ行く"),
        ];
        let group = group_burst(&recent, 5, WINDOW);
        assert_eq!(
            group.iter().map(|m| m.rowid).collect::<Vec<_>>(),
            vec![2, 3, 4, 5]
        );
        assert!(burst_text(&group).contains("資格確認証"));
        assert!(burst_text(&group).ends_with("返信なければ行く"));
    }

    /// 自分が返していたら、そこで話は切れている。
    /// またぐと、返事済みの用件にもう一度返すことになる。
    #[test]
    fn an_own_reply_breaks_the_burst() {
        let recent = vec![
            at(1, false, 0, "資格証明書はあるの？"),
            at(2, true, 60, "ある"),
            at(3, false, 120, "マイナンバーカードは？"),
            at(4, false, 130, "何故ですか？"),
        ];
        let group = group_burst(&recent, 4, WINDOW);
        assert_eq!(group.iter().map(|m| m.rowid).collect::<Vec<_>>(), vec![3, 4]);
    }

    /// 間隔が空いていれば別の用件。1 時間前の話まで巻き込まない。
    #[test]
    fn a_long_pause_breaks_the_burst() {
        let recent = vec![
            at(1, false, 0, "おはよう"),
            at(2, false, 3600, "ところで"),
            at(3, false, 3610, "今日来る？"),
        ];
        let group = group_burst(&recent, 3, WINDOW);
        assert_eq!(group.iter().map(|m| m.rowid).collect::<Vec<_>>(), vec![2, 3]);
    }

    /// 「👍」が 1 つ挟まっただけで分断されると、まとめる意味が無い。
    #[test]
    fn a_tapback_does_not_break_the_burst() {
        let mut tapback = at(2, false, 60, "");
        tapback.skip = Some(super::super::SkipReason::Tapback);
        let recent = vec![at(1, false, 0, "答えて"), tapback, at(3, false, 70, "何故？")];
        let group = group_burst(&recent, 3, WINDOW);
        assert_eq!(group.iter().map(|m| m.rowid).collect::<Vec<_>>(), vec![1, 3]);
    }

    #[test]
    fn a_single_message_is_a_burst_of_one() {
        let recent = vec![at(1, true, 0, "ある"), at(2, false, 60, "何故ですか？")];
        let group = group_burst(&recent, 2, WINDOW);
        assert_eq!(group.len(), 1);
        assert_eq!(burst_text(&group), "何故ですか？");
    }

    /// 長い独白がまるごと入ると生成が壊れる。
    #[test]
    fn a_burst_is_capped() {
        let recent: Vec<Message> = (1..=30)
            .map(|i| at(i, false, i * 10, &format!("行{i}")))
            .collect();
        assert_eq!(group_burst(&recent, 30, WINDOW).len(), BURST_MAX);
    }

    #[test]
    fn an_unknown_rowid_yields_nothing() {
        assert!(group_burst(&[at(1, false, 0, "a")], 99, WINDOW).is_empty());
    }

    /// 本文の無いものが混じっても、区切りが増えたりしない。
    #[test]
    fn empty_bodies_do_not_leave_blank_lines() {
        let mut blank = at(2, false, 10, "   ");
        blank.body = Some("   ".into());
        let group = vec![at(1, false, 0, "答えて"), blank, at(3, false, 20, "何故？")];
        assert_eq!(burst_text(&group), "答えて\n何故？");
    }

    /// 相手は 1 つの用件を数行に分けて送る。行ごとに返信すると会話が壊れる。
    #[test]
    fn only_the_newest_message_is_acted_on() {
        let plan = plan(vec![msg(1, false), msg(2, false), msg(3, false)], false);
        assert_eq!(plan.actionable.unwrap().rowid, 3);
        assert_eq!(plan.passed.len(), 2);
        assert!(plan.passed.iter().all(|(_, r)| *r == Passed::Superseded));
    }

    /// 時刻が飛んだあとは stale として記録する。あとから理由を追えるようにする。
    #[test]
    fn after_a_gap_the_rest_are_marked_stale() {
        let plan = plan(vec![msg(1, false), msg(2, false)], true);
        assert_eq!(plan.actionable.unwrap().rowid, 2);
        assert_eq!(plan.passed[0].1, Passed::Stale);
    }

    /// 自分の送信は絶対にトリガーにしない（仕様書 6.4.4 ループ防止）。
    #[test]
    fn own_messages_never_trigger_generation() {
        let plan = plan(vec![msg(1, false), msg(2, true)], false);
        assert_eq!(plan.actionable.unwrap().rowid, 1);
        assert_eq!(plan.passed[0].1, Passed::NotApplicable);
    }

    #[test]
    fn a_batch_of_only_own_messages_produces_nothing() {
        let plan = plan(vec![msg(1, true), msg(2, true)], false);
        assert!(plan.actionable.is_none());
        // それでも起点は進める。進めないと毎回同じ行を読み直す。
        assert_eq!(plan.next_seen_rowid, Some(2));
    }

    #[test]
    fn skipped_messages_are_not_actionable() {
        let mut tapback = msg(2, false);
        tapback.skip = Some(super::super::SkipReason::Tapback);
        let plan = plan(vec![msg(1, false), tapback], false);
        assert_eq!(plan.actionable.unwrap().rowid, 1);
    }

    #[test]
    fn an_empty_batch_leaves_the_cursor_alone() {
        let plan = plan(vec![], false);
        assert!(plan.actionable.is_none());
        assert_eq!(plan.next_seen_rowid, None);
    }

    // MARK: stale guard

    #[test]
    fn a_recent_message_is_not_stale() {
        let now = Local.with_ymd_and_hms(2026, 8, 1, 12, 0, 0).unwrap();
        let received = Local.with_ymd_and_hms(2026, 8, 1, 11, 55, 0).unwrap();
        assert!(!is_stale(received, now, Duration::from_secs(900)));
    }

    #[test]
    fn an_old_message_is_stale() {
        let now = Local.with_ymd_and_hms(2026, 8, 1, 12, 0, 0).unwrap();
        let received = Local.with_ymd_and_hms(2026, 8, 1, 11, 0, 0).unwrap();
        assert!(is_stale(received, now, Duration::from_secs(900)));
    }

    /// 時計のずれで受信時刻が未来になることがある。古い扱いにしない。
    #[test]
    fn a_future_timestamp_is_not_stale() {
        let now = Local.with_ymd_and_hms(2026, 8, 1, 12, 0, 0).unwrap();
        let received = Local.with_ymd_and_hms(2026, 8, 1, 12, 5, 0).unwrap();
        assert!(!is_stale(received, now, Duration::from_secs(900)));
    }

    // MARK: 時刻ギャップ

    #[test]
    fn a_long_pause_is_a_gap() {
        assert!(gap_detected(Some(1_000), 1_000 + 601, Duration::from_secs(600)));
        assert!(!gap_detected(Some(1_000), 1_000 + 60, Duration::from_secs(600)));
    }

    /// 初回はギャップ扱いしない。バックログ保護が別に効いている。
    #[test]
    fn the_first_poll_is_not_a_gap() {
        assert!(!gap_detected(None, 9_999_999, Duration::from_secs(600)));
    }

    #[test]
    fn wal_and_shm_changes_are_recognised() {
        let stem = std::ffi::OsString::from("chat.db");
        assert!(is_chat_db_file(Path::new("/x/chat.db"), &stem));
        assert!(is_chat_db_file(Path::new("/x/chat.db-wal"), &stem));
        assert!(is_chat_db_file(Path::new("/x/chat.db-shm"), &stem));
        assert!(!is_chat_db_file(Path::new("/x/other.db"), &stem));
        assert!(!is_chat_db_file(Path::new("/x/chat.dbx"), &stem));
    }
}
