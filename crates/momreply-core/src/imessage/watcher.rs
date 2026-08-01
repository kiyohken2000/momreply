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
//! 新着が複数あっても**生成対象は最新の 1 件だけ**にする。
//! 相手は 1 つの用件を数行に分けて送ってくるので、行ごとに返信すると
//! 会話が壊れる。古い分は `superseded`、時刻ギャップ後なら `stale`
//! として記録に残す。

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

use super::Message;

/// 見送った理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Passed {
    /// 同じ相手からより新しいメッセージが来ている。
    Superseded,
    /// 時刻が飛んだあとに溜まっていた分（仕様書 6.1）。
    Stale,
    /// 自分の送信・タップバックなど、そもそも対象外。
    NotApplicable,
}

impl Passed {
    pub fn label(self) -> &'static str {
        match self {
            Passed::Superseded => "superseded",
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
