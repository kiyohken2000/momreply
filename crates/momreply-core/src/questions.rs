//! 受信メッセージから質問を取り出す。
//!
//! 仕様書には無い機構。相手の質問に具体的に答えることを目的にすると、
//! 「何を聞かれているか」を本文とは別に持つ必要が出てくる。
//!
//! ここは LLM を使わない。理由は 2 つある。
//! - 呼び出し頻度が最も高く、課金が積み上がる（仕様書 7.3.2 も分類は
//!   オンデバイス側に寄せている）
//! - 質問の取りこぼしはテストで潰せる性質のもので、確率的な処理に
//!   任せる必要がない
//!
//! 曖昧な文の意味解釈は LLM 側（生成時）に任せ、ここは「疑問文の切り出し」
//! だけを担当する。

/// 疑問符で終わらない疑問文の語尾。
///
/// 相手が疑問符を省略することは多い（「いつ来るのか」「どうするつもり」）。
const QUESTION_SUFFIXES: [&str; 10] = [
    "ですか",
    "ますか",
    "のか",
    "だろうか",
    "でしょうか",
    "かな",
    "かね",
    "だっけ",
    "ますでしょうか",
    "いかが",
];

/// 本文から質問だけを取り出す。原文の表記のまま返す。
///
/// 改行区切りで書かれた 1 つの質問（「書類は」「あるの？」）を
/// 分断しないよう、終端記号が来るまで改行をまたいで連結する。
pub fn extract(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();

    for ch in body.chars() {
        match ch {
            '？' | '?' => {
                buf.push(ch);
                push_if_question(&mut out, &buf, true);
                buf.clear();
            }
            '。' | '！' | '!' => {
                buf.push(ch);
                push_if_question(&mut out, &buf, false);
                buf.clear();
            }
            '\n' => {
                // 改行だけでは切らない。語尾が疑問形になっていれば
                // そこで 1 文として確定させる。
                if is_question_by_suffix(buf.trim()) {
                    push_if_question(&mut out, &buf, false);
                    buf.clear();
                } else {
                    buf.push(' ');
                }
            }
            _ => buf.push(ch),
        }
    }
    push_if_question(&mut out, &buf, false);

    out
}

/// 同じ質問を二度人間に聞かないための正規化キー。
///
/// 吸収できるのは**句読点・空白・疑問符・丁寧語の語尾**まで。
/// 活用の揺れ（「ありますか」と「ある？」）は落とせない。ここを
/// 追うと形態素解析が要り、費用に見合わない。
///
/// 実データでは相手は**同じ文面をそのまま繰り返す**ことが多く、
/// その場合はこのキーで吸収できる。取りこぼしても影響は
/// 「同じ事実を二度聞かれる」だけで、誤った答えは出ない。
/// 意味の一致判定は生成時に LLM 側で `self.md` と突き合わせて行う。
pub fn normalize(question: &str) -> String {
    let stripped: String = question
        .chars()
        .filter(|c| !matches!(c, '？' | '?' | '。' | '、' | '，' | '！' | '!' | ' ' | '　' | '\n' | '\t'))
        .collect();

    let mut s = stripped.as_str();
    for suffix in ["ますでしょうか", "でしょうか", "ですか", "ますか", "のか", "かな", "かね"] {
        if let Some(rest) = s.strip_suffix(suffix) {
            if !rest.is_empty() {
                s = rest;
                break;
            }
        }
    }

    // 記号だけの質問を空キーにすると、無関係な質問が 1 つに潰れて
    // 「既に答えた」と誤判定される。原文に戻す。
    if s.is_empty() {
        return question.trim().to_string();
    }
    s.to_string()
}

fn push_if_question(out: &mut Vec<String>, buf: &str, explicit: bool) {
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        return;
    }
    if explicit || is_question_by_suffix(trimmed) {
        out.push(trimmed.to_string());
    }
}

fn is_question_by_suffix(s: &str) -> bool {
    let t = s.trim_end_matches(['。', '、', ' ', '　']);
    if t.is_empty() {
        return false;
    }
    QUESTION_SUFFIXES.iter().any(|suffix| t.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_up_explicit_questions() {
        assert_eq!(extract("保険証は、ありますか？"), vec!["保険証は、ありますか？"]);
        assert_eq!(extract("何故ですか？"), vec!["何故ですか？"]);
    }

    /// 相手は 1 つの質問を改行で分けて書く。ここで分断すると
    /// 「書類は」だけが質問として残り、意味が失われる。
    #[test]
    fn a_question_split_across_newlines_stays_whole() {
        assert_eq!(extract("書類は\nあるの？"), vec!["書類は あるの？"]);
    }

    #[test]
    fn ignores_statements() {
        assert!(extract("明日行くね\nそのとき、郵便物も\n持参するね").is_empty());
        assert!(extract("月曜日の夜行きます。").is_empty());
    }

    #[test]
    fn picks_up_questions_without_a_question_mark() {
        assert_eq!(extract("いつ取得する予定ですか"), vec!["いつ取得する予定ですか"]);
        assert_eq!(extract("どうするつもりなのか"), vec!["どうするつもりなのか"]);
    }

    #[test]
    fn separates_multiple_questions_in_one_message() {
        let got = extract("免許証は、いつ取得する予定ですか？\nあと保険証はある？");
        assert_eq!(
            got,
            vec!["免許証は、いつ取得する予定ですか？", "あと保険証はある？"]
        );
    }

    /// 同じことを繰り返し聞かれても、人間に聞くのは一度だけにしたい。
    /// 実データでは同じ文面がそのまま繰り返される（句読点だけ違う）ケースが多い。
    #[test]
    fn repeated_questions_collapse_to_one_key() {
        assert_eq!(
            normalize("保険証は、ありますか？"),
            normalize("保険証はありますか")
        );
        assert_eq!(normalize("何故ですか？"), normalize("何故"));
        assert_eq!(normalize("いつ取得しますか？"), normalize("いつ取得します　か"));
    }

    #[test]
    fn different_questions_keep_different_keys() {
        assert_ne!(normalize("保険証はある？"), normalize("免許証はある？"));
        assert_ne!(normalize("いつ来るの？"), normalize("なぜ来るの？"));
    }

    /// 既知の限界。活用の揺れは吸収できない。
    /// 影響は「同じ事実を二度聞かれる」だけで、誤答は出ない。
    /// 意味の一致は生成時に LLM が self.md と突き合わせて判定する。
    #[test]
    fn conjugation_differences_are_not_absorbed() {
        assert_ne!(normalize("保険証はありますか？"), normalize("保険証はある？"));
    }

    /// キーが空になると、無関係な質問どうしが同一視されて
    /// 「その質問には答え済み」と誤判定される。
    #[test]
    fn normalize_never_returns_an_empty_key() {
        assert!(!normalize("ですか？").is_empty());
        assert!(!normalize("？").is_empty());
    }
}
