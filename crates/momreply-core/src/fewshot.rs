//! 過去のやり取りから (相手の発言, 自分の返信) のペアを作る（仕様書 6.8）。
//!
//! # 何のために使うか
//!
//! **文体の再現にだけ使う。内容の根拠にはしない。**
//!
//! 仕様書 6.8 は「自分の過去返信を few-shot として利用する」とだけ書いているが、
//! 実データを見ると、過去の返信をそのまま手本にすると困る場合がある。
//! 過去に短く突き放した返し方をしていれば、それを再生産してしまう。
//!
//! このアプリの目的は「質問に具体的に答える」ことなので、
//! **語尾・句読点・絵文字の使い方といった話し方だけをここから借り、
//! 何を答えるかは `self.md` と定型回答から決める。**
//! その分担はプロンプト側（[`crate::pipeline::prompt`]）で明示している。

use anyhow::Result;
use rusqlite::Connection;

use crate::{imessage, store::Store};

/// 返信が短すぎるものは文体の手本にならない（「w」など）。
const MIN_REPLY_CHARS: usize = 2;
/// 長すぎる返信は例外的な事情のものが多い。
const MAX_REPLY_CHARS: usize = 200;
/// 同じ返信文が何度も並ぶと、その一言に引っ張られる。
const MAX_SAME_REPLY: usize = 3;
/// 走査するペア数の上限。
const SCAN_PAIRS: usize = 200;
/// 直近から必ず採る数。最新の文体を反映させる。
const RECENT_PAIRS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub incoming: String,
    pub reply: String,
    pub source_rowid: i64,
}

/// chat.db を走査してペアを作り、app.db に保存する。
///
/// 戻り値は保存した件数。
pub fn rebuild(
    chat_db: &Connection,
    store: &Store,
    target_id: i64,
    handles: &[String],
    limit: usize,
    scan_messages: u32,
) -> Result<usize> {
    let messages = imessage::recent_messages(chat_db, handles, scan_messages)?;
    let pairs = select(&build_pairs(&messages), limit);
    store.replace_fewshot(target_id, &pairs)?;
    Ok(pairs.len())
}

/// 「相手の発言」→「直後の自分の返信」を拾う。
///
/// 相手が連続して送ってきた場合は最後の発言を、自分が連続して返した場合は
/// 最初の返信を採る。会話の切れ目で対応が崩れないようにするため。
pub fn build_pairs(messages: &[imessage::Message]) -> Vec<Pair> {
    let mut pairs = Vec::new();
    let mut pending: Option<&imessage::Message> = None;

    for m in messages {
        if m.skip.is_some() {
            continue;
        }
        let Some(body) = m.body.as_deref() else {
            continue;
        };

        if m.is_from_me {
            if let Some(incoming) = pending.take() {
                pairs.push(Pair {
                    incoming: incoming.body.clone().unwrap_or_default(),
                    reply: body.to_string(),
                    source_rowid: m.rowid,
                });
            }
        } else {
            // 相手が続けて送ってきたら、直前のものは上書きする。
            pending = Some(m);
        }
    }

    pairs.retain(usable);
    pairs
}

fn usable(p: &Pair) -> bool {
    let reply = p.reply.chars().count();
    reply >= MIN_REPLY_CHARS && reply <= MAX_REPLY_CHARS && !p.incoming.trim().is_empty()
}

/// 直近ぶんと、それ以前から均等に間引いたぶんを混ぜて選ぶ。
///
/// 仕様書 6.8 は「残りからランダムに」としているが、等間隔で拾う。
/// 乱数だと実行のたびに結果が変わり、生成が不安定になった理由を
/// 追えなくなる。等間隔でも期間の広がりは確保できる。
pub fn select(pairs: &[Pair], limit: usize) -> Vec<Pair> {
    let pairs = if pairs.len() > SCAN_PAIRS {
        &pairs[pairs.len() - SCAN_PAIRS..]
    } else {
        pairs
    };
    if pairs.len() <= limit {
        return dedupe(pairs.to_vec());
    }

    let recent_n = RECENT_PAIRS.min(limit).min(pairs.len());
    let split = pairs.len() - recent_n;
    let (older, recent) = pairs.split_at(split);

    let want_older = limit - recent_n;
    let mut chosen = Vec::with_capacity(limit);
    if want_older > 0 && !older.is_empty() {
        let stride = older.len() as f64 / want_older as f64;
        for i in 0..want_older {
            let idx = ((i as f64) * stride).floor() as usize;
            if let Some(p) = older.get(idx.min(older.len() - 1)) {
                chosen.push(p.clone());
            }
        }
    }
    chosen.extend(recent.iter().cloned());
    dedupe(chosen)
}

/// 同じ返信文が並びすぎるのを防ぐ（仕様書 6.8「うんの偏り防止」）。
fn dedupe(pairs: Vec<Pair>) -> Vec<Pair> {
    let mut seen: Vec<(String, usize)> = Vec::new();
    let mut out = Vec::with_capacity(pairs.len());

    for p in pairs {
        let key = p.reply.trim().to_string();
        match seen.iter_mut().find(|(k, _)| *k == key) {
            Some((_, n)) if *n >= MAX_SAME_REPLY => continue,
            Some((_, n)) => *n += 1,
            None => seen.push((key, 1)),
        }
        out.push(p);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;

    fn msg(rowid: i64, from_me: bool, body: &str) -> imessage::Message {
        imessage::Message {
            rowid,
            guid: format!("g{rowid}"),
            chat_identifier: "x@example.com".into(),
            date: Local::now(),
            is_from_me: from_me,
            edited: false,
            body: Some(body.to_string()),
            skip: None,
            body_from_text_column: false,
        }
    }

    fn pair(incoming: &str, reply: &str, rowid: i64) -> Pair {
        Pair {
            incoming: incoming.into(),
            reply: reply.into(),
            source_rowid: rowid,
        }
    }

    #[test]
    fn pairs_an_incoming_message_with_the_next_reply() {
        let msgs = vec![
            msg(1, false, "ごはん食べた？"),
            msg(2, true, "食べたよー"),
            msg(3, false, "明日雨だって"),
            msg(4, true, "まじか、傘持ってく"),
        ];
        let pairs = build_pairs(&msgs);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].incoming, "ごはん食べた？");
        assert_eq!(pairs[0].reply, "食べたよー");
        assert_eq!(pairs[1].source_rowid, 4);
    }

    /// 相手が続けて送ってきたときは、返信が答えている最後の発言と組む。
    #[test]
    fn consecutive_incoming_messages_use_the_last_one() {
        let msgs = vec![
            msg(1, false, "おはよう"),
            msg(2, false, "今日くる？"),
            msg(3, true, "行かない"),
        ];
        let pairs = build_pairs(&msgs);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].incoming, "今日くる？");
    }

    /// 自分が続けて送った場合、2 通目は独立したペアにしない。
    #[test]
    fn consecutive_replies_only_produce_one_pair() {
        let msgs = vec![
            msg(1, false, "元気？"),
            msg(2, true, "元気だよ"),
            msg(3, true, "そっちは？"),
        ];
        assert_eq!(build_pairs(&msgs).len(), 1);
    }

    #[test]
    fn too_short_and_too_long_replies_are_dropped() {
        let msgs = vec![
            msg(1, false, "ねえ"),
            msg(2, true, "w"),
            msg(3, false, "どう？"),
            msg(4, true, &"あ".repeat(MAX_REPLY_CHARS + 1)),
            msg(5, false, "ほんと？"),
            msg(6, true, "ほんとだよ"),
        ];
        let pairs = build_pairs(&msgs);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].reply, "ほんとだよ");
    }

    #[test]
    fn skipped_messages_never_become_pairs() {
        let mut tapback = msg(2, false, "いいね");
        tapback.skip = Some(imessage::SkipReason::Tapback);
        let msgs = vec![msg(1, false, "見て"), tapback, msg(3, true, "見たよー")];
        let pairs = build_pairs(&msgs);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].incoming, "見て");
    }

    /// 同じ返信ばかり並ぶと、その一言に引っ張られて何を聞かれても
    /// それを返すようになる。
    #[test]
    fn the_same_reply_appears_at_most_three_times() {
        let pairs: Vec<Pair> = (0..10).map(|i| pair("なにしてる？", "うん", i)).collect();
        assert_eq!(dedupe(pairs).len(), MAX_SAME_REPLY);
    }

    #[test]
    fn different_replies_are_all_kept() {
        let pairs = vec![
            pair("a", "うん", 1),
            pair("b", "そうだね", 2),
            pair("c", "わかった", 3),
        ];
        assert_eq!(dedupe(pairs).len(), 3);
    }

    #[test]
    fn selection_keeps_the_most_recent_pairs() {
        let pairs: Vec<Pair> = (0..100)
            .map(|i| pair("q", &format!("返信{i}"), i))
            .collect();
        let chosen = select(&pairs, 40);
        assert_eq!(chosen.len(), 40);
        // 直近 20 件は必ず入る。
        assert!(chosen.iter().any(|p| p.reply == "返信99"));
        assert!(chosen.iter().any(|p| p.reply == "返信80"));
        // 古いほうからも拾えている。
        assert!(chosen.iter().any(|p| p.source_rowid < 40));
    }

    /// 同じ入力なら同じ結果になること。乱数で選ぶと、生成が変わった
    /// 理由が few-shot なのかプロンプトなのか切り分けられなくなる。
    #[test]
    fn selection_is_deterministic() {
        let pairs: Vec<Pair> = (0..100)
            .map(|i| pair("q", &format!("返信{i}"), i))
            .collect();
        assert_eq!(select(&pairs, 40), select(&pairs, 40));
    }

    #[test]
    fn fewer_pairs_than_requested_is_fine() {
        let pairs = vec![pair("a", "うん", 1), pair("b", "そうだね", 2)];
        assert_eq!(select(&pairs, 40).len(), 2);
    }
}
