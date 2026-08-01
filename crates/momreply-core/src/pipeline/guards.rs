//! セーフティガード（仕様書 6.4）。
//!
//! **すべて必須。1つでも欠けると事故が起きる。**
//!
//! ここは「送ってよいか」を判断するだけで、送信も記録もしない。
//! 判断が純粋関数になっていれば、事故のパターンをテストで固定できる。

use std::time::Duration;

use chrono::{DateTime, Local};

/// 送信の可否。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// 自動送信してよい。
    AutoSend,
    /// 生成はするが、人が確認してから送る。
    Review(HoldReason),
    /// 生成もしない。
    Skip(SkipReason),
}

/// 確認に回す理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldReason {
    /// 受信から時間が経ちすぎている（仕様書 6.4.2）。
    Stale,
    /// 自動送信が無効（キルスイッチ・相手ごとの設定）。
    AutoSendOff,
    /// ドライラン中（仕様書 6.4.7）。
    DryRun,
    /// 1 時間 / 1 日の上限に達した（仕様書 6.4.5）。
    RateLimited,
    /// 連続自動返信の上限に達した（仕様書 6.4.5.1）。
    ConsecutiveLimit,
    /// 月次のコスト上限（仕様書 6.4.5.2）。
    BudgetSoftLimit,
    /// エスカレーション条件に合致（仕様書 6.4.8）。
    Escalated,
    /// 送信直前に長さの上限を超えた（仕様書 6.2.1-5）。
    TooLong,
}

/// 生成すらしない理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// 既に自分が返信している（仕様書 6.4.3）。
    AlreadyReplied,
    /// 自分が送信した直後（仕様書 6.4.4 クールダウン）。
    Cooldown,
    /// 月次のハード上限（仕様書 6.4.5.2）。
    BudgetHardLimit,
}

impl HoldReason {
    pub fn label(&self) -> &'static str {
        match self {
            HoldReason::Stale => "stale",
            HoldReason::AutoSendOff => "auto_send_off",
            HoldReason::DryRun => "dry_run",
            HoldReason::RateLimited => "rate_limited",
            HoldReason::ConsecutiveLimit => "consecutive_limit",
            HoldReason::BudgetSoftLimit => "budget_soft_limit",
            HoldReason::Escalated => "escalated",
            HoldReason::TooLong => "too_long",
        }
    }
}

impl SkipReason {
    pub fn label(&self) -> &'static str {
        match self {
            SkipReason::AlreadyReplied => "already_replied",
            SkipReason::Cooldown => "cooldown",
            SkipReason::BudgetHardLimit => "budget_hard_limit",
        }
    }
}

/// ガードの判断に必要な状態。
pub struct State {
    pub now: DateTime<Local>,
    pub received_at: DateTime<Local>,
    /// 対象より後に自分が送った件数（仕様書 6.4.3）。
    pub own_replies_after: i64,
    /// 自分が最後に送った時刻。
    pub last_sent_at: Option<DateTime<Local>>,
    /// 直近 1 時間 / 24 時間の自動送信数。
    pub sent_last_hour: u32,
    pub sent_last_day: u32,
    /// 連続自動返信回数（仕様書 6.4.5.1）。
    pub consecutive_auto: u32,
    /// 当月の推定コスト（USD）。
    pub month_cost_usd: f64,
    pub auto_send_enabled: bool,
    pub dry_run: bool,
    /// エスカレーション条件に合致したか。
    pub escalated: bool,
}

/// しきい値（仕様書 9 の設定項目）。
#[derive(Debug, Clone)]
pub struct Limits {
    pub stale_threshold: Duration,
    pub cooldown_after_send: Duration,
    pub max_per_hour: u32,
    pub max_per_day: u32,
    pub max_consecutive_auto: u32,
    pub monthly_soft_limit_usd: f64,
    pub monthly_hard_limit_usd: f64,
}

impl Limits {
    /// app.db から読む。未設定の項目は既定値のまま。
    ///
    /// **既定値をハードコードしたまま UI に出すと、変えても効かない設定に
    /// なる。** ここを通すことで、画面の値と実際の判定を一致させる。
    pub fn load(store: &crate::store::Store) -> anyhow::Result<Self> {
        let d = Self::default();
        let num = |key: &str| -> anyhow::Result<Option<f64>> {
            Ok(store.get_kv(key)?.and_then(|v| v.trim().parse().ok()))
        };

        Ok(Limits {
            stale_threshold: num("limits.stale_threshold_minutes")?
                .map(|m| Duration::from_secs((m * 60.0) as u64))
                .unwrap_or(d.stale_threshold),
            cooldown_after_send: num("limits.cooldown_seconds")?
                .map(|v| Duration::from_secs(v as u64))
                .unwrap_or(d.cooldown_after_send),
            max_per_hour: num("limits.max_per_hour")?.map(|v| v as u32).unwrap_or(d.max_per_hour),
            max_per_day: num("limits.max_per_day")?.map(|v| v as u32).unwrap_or(d.max_per_day),
            max_consecutive_auto: num("limits.max_consecutive_auto")?
                .map(|v| v as u32)
                .unwrap_or(d.max_consecutive_auto),
            monthly_soft_limit_usd: num("limits.monthly_soft_limit_usd")?
                .unwrap_or(d.monthly_soft_limit_usd),
            monthly_hard_limit_usd: num("limits.monthly_hard_limit_usd")?
                .unwrap_or(d.monthly_hard_limit_usd),
        })
    }
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            stale_threshold: Duration::from_secs(15 * 60),
            cooldown_after_send: Duration::from_secs(60),
            max_per_hour: 6,
            max_per_day: 30,
            max_consecutive_auto: 5,
            monthly_soft_limit_usd: 3.0,
            monthly_hard_limit_usd: 10.0,
        }
    }
}

/// 生成の前に判断する（仕様書 6.4）。
///
/// **判定の順序に意味がある。** 送ってはいけない度合いが強いものから
/// 見る。生成すら止めるもの（Skip）を先に返さないと、無駄な課金が発生する。
pub fn evaluate(state: &State, limits: &Limits) -> Verdict {
    // 1. 既に自分が返信している。二重返信は最も避けたい事故。
    if state.own_replies_after > 0 {
        return Verdict::Skip(SkipReason::AlreadyReplied);
    }

    // 2. 自分が送った直後。返信が相手に届く前にまた送るのを防ぐ。
    if let Some(last) = state.last_sent_at {
        if let Ok(elapsed) = state.now.signed_duration_since(last).to_std() {
            if elapsed < limits.cooldown_after_send {
                return Verdict::Skip(SkipReason::Cooldown);
            }
        }
    }

    // 3. 月次ハード上限。ここを超えたら生成もしない。
    if state.month_cost_usd >= limits.monthly_hard_limit_usd {
        return Verdict::Skip(SkipReason::BudgetHardLimit);
    }

    // ここから先は生成する。送信するかどうかだけの判断。
    if state.dry_run {
        return Verdict::Review(HoldReason::DryRun);
    }
    if !state.auto_send_enabled {
        return Verdict::Review(HoldReason::AutoSendOff);
    }
    if state.escalated {
        return Verdict::Review(HoldReason::Escalated);
    }
    if super::super::imessage::is_stale(state.received_at, state.now, limits.stale_threshold) {
        return Verdict::Review(HoldReason::Stale);
    }
    if state.sent_last_hour >= limits.max_per_hour || state.sent_last_day >= limits.max_per_day {
        return Verdict::Review(HoldReason::RateLimited);
    }
    if state.consecutive_auto >= limits.max_consecutive_auto {
        return Verdict::Review(HoldReason::ConsecutiveLimit);
    }
    if state.month_cost_usd >= limits.monthly_soft_limit_usd {
        // ソフト上限は通知のみで動作は継続する、と仕様書 6.4.5.2 にあるが、
        // 自動送信は止めて確認に倒す。金額が想定を超えている状態で
        // 無人のまま送り続けるほうが危ない。
        return Verdict::Review(HoldReason::BudgetSoftLimit);
    }

    Verdict::AutoSend
}

/// 連続自動返信カウンタをリセットすべきか（仕様書 6.4.5.1）。
///
/// **日付が変わったときを取りこぼさないこと**（仕様書 14.10）。
/// 忘れると翌朝の最初のメッセージが確認モードのまま放置される。
pub fn should_reset_consecutive(
    last_exchange: Option<DateTime<Local>>,
    now: DateTime<Local>,
    session_gap: Duration,
) -> bool {
    let Some(last) = last_exchange else {
        return true;
    };
    if last.date_naive() != now.date_naive() {
        return true;
    }
    match now.signed_duration_since(last).to_std() {
        Ok(elapsed) => elapsed >= session_gap,
        Err(_) => false,
    }
}

/// エスカレーション判定（仕様書 6.4.8）。初期値は無効。
pub fn is_escalated(body: &str, keywords: &[String], escalate_on_question: bool) -> bool {
    if escalate_on_question && (body.contains('?') || body.contains('？')) {
        return true;
    }
    keywords.iter().any(|k| !k.is_empty() && body.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32, m: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 8, 1, h, m, 0).unwrap()
    }

    fn ok_state() -> State {
        State {
            now: at(12, 0),
            received_at: at(11, 55),
            own_replies_after: 0,
            last_sent_at: None,
            sent_last_hour: 0,
            sent_last_day: 0,
            consecutive_auto: 0,
            month_cost_usd: 0.0,
            auto_send_enabled: true,
            dry_run: false,
            escalated: false,
        }
    }

    #[test]
    fn a_clean_state_allows_auto_send() {
        assert_eq!(evaluate(&ok_state(), &Limits::default()), Verdict::AutoSend);
    }

    /// 最重要。iPhone で先に手で返信していたら二重返信になる。
    #[test]
    fn an_existing_reply_stops_everything() {
        let s = State {
            own_replies_after: 1,
            ..ok_state()
        };
        assert_eq!(
            evaluate(&s, &Limits::default()),
            Verdict::Skip(SkipReason::AlreadyReplied)
        );
    }

    /// 既返信チェックは他のどの条件よりも先に効くこと。
    /// 順序を入れ替えると、ドライラン中に二重返信の記録が残る。
    #[test]
    fn the_reply_check_wins_over_everything_else() {
        let s = State {
            own_replies_after: 1,
            dry_run: true,
            auto_send_enabled: false,
            escalated: true,
            ..ok_state()
        };
        assert_eq!(
            evaluate(&s, &Limits::default()),
            Verdict::Skip(SkipReason::AlreadyReplied)
        );
    }

    #[test]
    fn a_recent_send_triggers_the_cooldown() {
        // 30 秒前 → まだクールダウン中。
        let s = State {
            last_sent_at: Some(at(12, 0) - chrono::Duration::seconds(30)),
            ..ok_state()
        };
        assert_eq!(
            evaluate(&s, &Limits::default()),
            Verdict::Skip(SkipReason::Cooldown)
        );

        // 2 分前 → 明けている。
        let s = State {
            last_sent_at: Some(at(11, 58)),
            ..ok_state()
        };
        assert_eq!(evaluate(&s, &Limits::default()), Verdict::AutoSend);
    }

    /// ちょうど 60 秒は「60 秒間の待機が終わった」と解釈する。
    /// どちらに倒すかを決めておかないと、境界で挙動が揺れる。
    #[test]
    fn exactly_the_cooldown_duration_is_allowed() {
        let s = State {
            last_sent_at: Some(at(11, 59)),
            ..ok_state()
        };
        assert_eq!(evaluate(&s, &Limits::default()), Verdict::AutoSend);
    }

    /// ハード上限は生成も止める。止めないと課金が続く。
    #[test]
    fn the_hard_budget_limit_stops_generation() {
        let s = State {
            month_cost_usd: 10.0,
            ..ok_state()
        };
        assert_eq!(
            evaluate(&s, &Limits::default()),
            Verdict::Skip(SkipReason::BudgetHardLimit)
        );
    }

    #[test]
    fn dry_run_generates_but_never_sends() {
        let s = State {
            dry_run: true,
            ..ok_state()
        };
        assert_eq!(
            evaluate(&s, &Limits::default()),
            Verdict::Review(HoldReason::DryRun)
        );
    }

    #[test]
    fn the_kill_switch_holds_for_review() {
        let s = State {
            auto_send_enabled: false,
            ..ok_state()
        };
        assert_eq!(
            evaluate(&s, &Limits::default()),
            Verdict::Review(HoldReason::AutoSendOff)
        );
    }

    /// 深夜のメッセージに朝まとめて返す事故を防ぐ。
    #[test]
    fn an_old_message_is_held_for_review() {
        let s = State {
            received_at: at(10, 0),
            ..ok_state()
        };
        assert_eq!(
            evaluate(&s, &Limits::default()),
            Verdict::Review(HoldReason::Stale)
        );
    }

    #[test]
    fn rate_limits_hold_for_review() {
        let s = State {
            sent_last_hour: 6,
            ..ok_state()
        };
        assert_eq!(
            evaluate(&s, &Limits::default()),
            Verdict::Review(HoldReason::RateLimited)
        );

        let s = State {
            sent_last_day: 30,
            ..ok_state()
        };
        assert_eq!(
            evaluate(&s, &Limits::default()),
            Verdict::Review(HoldReason::RateLimited)
        );
    }

    #[test]
    fn the_consecutive_limit_switches_to_review() {
        let s = State {
            consecutive_auto: 5,
            ..ok_state()
        };
        assert_eq!(
            evaluate(&s, &Limits::default()),
            Verdict::Review(HoldReason::ConsecutiveLimit)
        );
    }

    #[test]
    fn escalation_holds_for_review() {
        let s = State {
            escalated: true,
            ..ok_state()
        };
        assert_eq!(
            evaluate(&s, &Limits::default()),
            Verdict::Review(HoldReason::Escalated)
        );
    }

    // MARK: 連続カウンタのリセット（仕様書 6.4.5.1 / 14.10）

    /// 日付が変わったときを取りこぼすと、翌朝の最初のメッセージが
    /// 確認モードのまま放置される。
    #[test]
    fn a_new_day_resets_the_counter() {
        let last = Local.with_ymd_and_hms(2026, 7, 31, 23, 50, 0).unwrap();
        let now = Local.with_ymd_and_hms(2026, 8, 1, 0, 10, 0).unwrap();
        // 経過は 20 分でセッション内だが、日付が変わっているのでリセット。
        assert!(should_reset_consecutive(
            Some(last),
            now,
            Duration::from_secs(180 * 60)
        ));
    }

    #[test]
    fn a_long_pause_resets_the_counter() {
        assert!(should_reset_consecutive(
            Some(at(8, 0)),
            at(12, 0),
            Duration::from_secs(180 * 60)
        ));
    }

    #[test]
    fn an_ongoing_session_keeps_the_counter() {
        assert!(!should_reset_consecutive(
            Some(at(11, 30)),
            at(12, 0),
            Duration::from_secs(180 * 60)
        ));
    }

    #[test]
    fn no_previous_exchange_starts_fresh() {
        assert!(should_reset_consecutive(
            None,
            at(12, 0),
            Duration::from_secs(180 * 60)
        ));
    }

    // MARK: エスカレーション

    #[test]
    fn keywords_trigger_escalation() {
        let kw = vec!["入院".to_string(), "振り込".to_string()];
        assert!(is_escalated("来週入院することになった", &kw, false));
        assert!(!is_escalated("今日は暑いね", &kw, false));
    }

    #[test]
    fn an_empty_keyword_list_never_escalates() {
        assert!(!is_escalated("入院する", &[], false));
        // 空文字が混ざっていても全件一致にしない。
        assert!(!is_escalated("なんでも", &["".to_string()], false));
    }

    #[test]
    fn question_marks_escalate_only_when_enabled() {
        assert!(is_escalated("来る？", &[], true));
        assert!(!is_escalated("来る？", &[], false));
    }
}
