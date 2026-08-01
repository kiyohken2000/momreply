//! 受信 → 生成 → （送信）のオーケストレーション。
//!
//! 送信は Phase 2 で足す。ここまでは生成と記録だけを行う。

pub mod generate;
pub mod guards;
pub mod prompt;
pub mod run;

pub use generate::{draft_reply, Draft, LengthPreset, Redo, Urgency};
pub use guards::{evaluate, HoldReason, Limits, Verdict};
pub use prompt::{Context, Retry};
pub use run::{process, Options, Outcome};

/// 生成結果の後処理の結末（仕様書 6.2.1）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cleaned {
    /// 送信候補として使える。
    Ok(String),
    /// 長さが上限を超えた。暴走の疑いがあるので**送らず**確認に回す。
    TooLong { text: String, chars: usize },
    /// 空。生成失敗として扱う。
    Empty,
}

/// LLM の出力を整える（仕様書 6.2.1）。
///
/// 1. 前後の空白・改行をトリム
/// 2. 全体を囲むクォートを除去
/// 3. 行頭の前置きを除去
/// 4. コードブロックの中身を取り出す
/// 5. `hard_max_length` 超過は送信せず確認へ倒す
/// 6. 空文字は失敗
pub fn clean(raw: &str, hard_max_length: usize) -> Cleaned {
    let mut text = raw.trim().to_string();

    // コードブロックが先。中にクォートや前置きが入っていることがある。
    text = strip_code_fence(&text);
    text = strip_preamble(&text);
    text = strip_wrapping_quotes(&text);
    let text = text.trim().to_string();

    if text.is_empty() {
        return Cleaned::Empty;
    }

    let chars = text.chars().count();
    if chars > hard_max_length {
        return Cleaned::TooLong { text, chars };
    }
    Cleaned::Ok(text)
}

fn strip_code_fence(s: &str) -> String {
    let t = s.trim();
    if !t.starts_with("```") {
        return s.to_string();
    }
    let mut lines: Vec<&str> = t.lines().collect();
    lines.remove(0); // ```lang
    if lines.last().map(|l| l.trim() == "```").unwrap_or(false) {
        lines.pop();
    }
    lines.join("\n")
}

/// 行頭の前置きを落とす。
///
/// 本文中のコロンは消さない。「明日は10:30に行くね」を壊さないため、
/// **既知の前置きで始まる場合だけ**削る。
fn strip_preamble(s: &str) -> String {
    const PREFIXES: [&str; 8] = [
        "返信:",
        "返信：",
        "返信文:",
        "返信文：",
        "以下のように返信します:",
        "以下のように返信します：",
        "回答:",
        "回答：",
    ];
    let t = s.trim_start();
    for p in PREFIXES {
        if let Some(rest) = t.strip_prefix(p) {
            return rest.trim_start().to_string();
        }
    }
    s.to_string()
}

/// 全体を囲むクォートだけを外す。
///
/// 会話の引用として途中に出てくる「」は残す。開きと閉じが
/// 両端にあり、かつ途中で閉じていない場合だけ外す。
fn strip_wrapping_quotes(s: &str) -> String {
    const PAIRS: [(char, char); 4] = [('"', '"'), ('“', '”'), ('「', '」'), ('『', '』')];
    let t = s.trim();
    let chars: Vec<char> = t.chars().collect();
    if chars.len() < 2 {
        return s.to_string();
    }

    for (open, close) in PAIRS {
        if chars[0] != open || chars[chars.len() - 1] != close {
            continue;
        }
        // 途中で閉じているなら、全体を囲んでいるわけではない。
        let inner = &chars[1..chars.len() - 1];
        let mut depth = 0i32;
        let mut closes_early = false;
        for &c in inner {
            if c == open && open != close {
                depth += 1;
            } else if c == close {
                if depth == 0 {
                    closes_early = true;
                    break;
                }
                depth -= 1;
            }
        }
        if !closes_early {
            return inner.iter().collect::<String>().trim().to_string();
        }
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: usize = 300;

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(clean("大丈夫だよ", MAX), Cleaned::Ok("大丈夫だよ".into()));
        assert_eq!(clean("  はい  \n", MAX), Cleaned::Ok("はい".into()));
    }

    #[test]
    fn wrapping_quotes_are_removed() {
        assert_eq!(clean("「行かないよ」", MAX), Cleaned::Ok("行かないよ".into()));
        assert_eq!(clean("\"行かないよ\"", MAX), Cleaned::Ok("行かないよ".into()));
    }

    /// 会話の引用まで外すと意味が変わる。
    #[test]
    fn quotes_inside_the_text_are_kept() {
        let s = "「行く」って言ったっけ？";
        assert_eq!(clean(s, MAX), Cleaned::Ok(s.into()));

        let s2 = "「うん」と答えたけど「やっぱり」と思った";
        assert_eq!(clean(s2, MAX), Cleaned::Ok(s2.into()));
    }

    #[test]
    fn preambles_are_removed() {
        assert_eq!(clean("返信: 行かない", MAX), Cleaned::Ok("行かない".into()));
        assert_eq!(
            clean("以下のように返信します：\n行かない", MAX),
            Cleaned::Ok("行かない".into())
        );
    }

    /// 本文中のコロンを前置きと誤認しない。
    #[test]
    fn a_colon_in_the_body_is_not_a_preamble() {
        let s = "明日は10:30に行くね";
        assert_eq!(clean(s, MAX), Cleaned::Ok(s.into()));
    }

    #[test]
    fn code_fences_are_unwrapped() {
        assert_eq!(
            clean("```\n行かないよ\n```", MAX),
            Cleaned::Ok("行かないよ".into())
        );
        assert_eq!(
            clean("```text\n行かないよ\n```", MAX),
            Cleaned::Ok("行かないよ".into())
        );
    }

    #[test]
    fn a_fence_wrapping_a_quoted_preamble_is_fully_unwrapped() {
        assert_eq!(
            clean("```\n返信:「行かない」\n```", MAX),
            Cleaned::Ok("行かない".into())
        );
    }

    /// 上限超過は送らない。暴走検知であって目標値ではない（仕様書 6.9.1）。
    #[test]
    fn too_long_output_is_held_for_review() {
        let long = "あ".repeat(MAX + 1);
        match clean(&long, MAX) {
            Cleaned::TooLong { chars, .. } => assert_eq!(chars, MAX + 1),
            other => panic!("確認に倒れていない: {other:?}"),
        }
    }

    #[test]
    fn exactly_the_limit_is_allowed() {
        let text = "あ".repeat(MAX);
        assert_eq!(clean(&text, MAX), Cleaned::Ok(text));
    }

    #[test]
    fn empty_output_is_a_failure() {
        assert_eq!(clean("", MAX), Cleaned::Empty);
        assert_eq!(clean("   \n  ", MAX), Cleaned::Empty);
        assert_eq!(clean("```\n\n```", MAX), Cleaned::Empty);
    }

    /// 文字数は書記素ではなく char で数える。日本語で桁を間違えない。
    #[test]
    fn length_is_counted_in_characters_not_bytes() {
        let text = "あ".repeat(100); // 300 バイト
        assert_eq!(clean(&text, 100), Cleaned::Ok(text));
    }
}
