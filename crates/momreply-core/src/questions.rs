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

/// 状況説明と質問を切り分ける閾値（文字数）。
///
/// 相手は「状況を数行書いてから最後に一言聞く」書き方をする。
/// 全体を質問として扱うと、毎回文面が変わるため重複判定が効かない。
/// 一方で短い文を切ると「書類は / あるの？」が「あるの？」だけになり
/// 意味が失われる。実データでは 40 字前後が境目だった。
const CONTEXT_SPLIT_THRESHOLD: usize = 40;

/// 質問の種類。答えの出どころが変わる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionKind {
    /// 自分が来るか・泊まるかを聞かれている。答えはその都度変わるため
    /// `self.md` の事実にはできない。定型回答（standing answer）を使う。
    Visit,
    /// それ以外。`self.md` の事実で答える。材料が無ければ人間に聞く。
    Fact,
}

/// 訪問を尋ねる語幹。
///
/// **「行く」系は入れない。** 相手から見た自分の訪問は必ず「来る」であり、
/// 「行く」は相手自身の行動か物のやり取り（「持って行けない」）を指す。
/// 入れると誤分類して、無関係な質問に定型回答を返してしまう。
const VISIT_STEMS: [&str; 9] = [
    "来る",
    "くる",
    "来ます",
    "来れ",
    "来られ",
    "来ない",
    "こない",
    "来い",
    "泊ま",
];

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

/// 取り出した質問。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// 質問そのもの。重複判定と人間への提示に使う。
    pub text: String,
    /// 質問の前に置かれた状況説明。生成時の文脈に使う。
    pub context: Option<String>,
}

impl Question {
    fn new(text: impl Into<String>) -> Self {
        Question {
            text: text.into(),
            context: None,
        }
    }

    pub fn kind(&self) -> QuestionKind {
        classify(&self.text)
    }
}

/// 本文から質問だけを取り出す。原文の表記のまま返す。
///
/// 改行区切りで書かれた 1 つの質問（「書類は」「あるの？」）を
/// 分断しないよう、終端記号が来るまで改行をまたいで連結する。
/// 連結結果が長くなった場合は、状況説明と質問に切り分ける。
pub fn extract(body: &str) -> Vec<Question> {
    let mut out = Vec::new();
    // 改行位置を保ったまま溜める。状況説明の切り出しに使う。
    let mut lines: Vec<String> = Vec::new();
    let mut buf = String::new();

    for ch in body.chars() {
        match ch {
            '？' | '?' => {
                buf.push(ch);
                lines.push(std::mem::take(&mut buf));
                push_if_question(&mut out, &lines, true);
                lines.clear();
            }
            '。' | '！' | '!' => {
                buf.push(ch);
                lines.push(std::mem::take(&mut buf));
                push_if_question(&mut out, &lines, false);
                lines.clear();
            }
            '\n' => {
                // 改行だけでは切らない。語尾が疑問形になっていれば
                // そこで 1 文として確定させる。
                lines.push(std::mem::take(&mut buf));
                if is_question_by_suffix(lines.last().map(|s| s.trim()).unwrap_or("")) {
                    push_if_question(&mut out, &lines, false);
                    lines.clear();
                }
            }
            _ => buf.push(ch),
        }
    }
    if !buf.trim().is_empty() {
        lines.push(buf);
    }
    push_if_question(&mut out, &lines, false);

    out
}

/// 質問を種類で分ける。
///
/// 迷ったら [`QuestionKind::Fact`] に倒す。`Fact` は材料が無ければ
/// 人間に聞きにいくため、誤っても誤答にはならない。一方 `Visit` の
/// 誤判定は、無関係な質問に定型回答を自動送信することになる。
pub fn classify(question: &str) -> QuestionKind {
    if VISIT_STEMS.iter().any(|stem| question.contains(stem)) {
        return QuestionKind::Visit;
    }
    QuestionKind::Fact
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

fn push_if_question(out: &mut Vec<Question>, lines: &[String], explicit: bool) {
    let joined = lines
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if joined.is_empty() {
        return;
    }
    if !explicit && !is_question_by_suffix(&joined) {
        return;
    }

    // 短ければ丸ごと質問。切ると意味が落ちる。
    if joined.chars().count() <= CONTEXT_SPLIT_THRESHOLD {
        out.push(Question::new(joined));
        return;
    }

    // 長い場合は最後の行を質問、それより前を状況説明とする。
    let non_empty: Vec<&str> = lines
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    match non_empty.split_last() {
        Some((last, before)) if !before.is_empty() => out.push(Question {
            text: (*last).to_string(),
            context: Some(before.join(" ")),
        }),
        _ => out.push(Question::new(joined)),
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

    fn texts(body: &str) -> Vec<String> {
        extract(body).into_iter().map(|q| q.text).collect()
    }

    #[test]
    fn picks_up_explicit_questions() {
        assert_eq!(texts("保険証は、ありますか？"), vec!["保険証は、ありますか？"]);
        assert_eq!(texts("何故ですか？"), vec!["何故ですか？"]);
    }

    /// 相手は 1 つの質問を改行で分けて書く。ここで分断すると
    /// 「書類は」だけが質問として残り、意味が失われる。
    #[test]
    fn a_short_question_split_across_newlines_stays_whole() {
        assert_eq!(texts("書類は\nあるの？"), vec!["書類は あるの？"]);
    }

    /// 状況を数行書いてから最後に一言聞く形。全体を質問にすると
    /// 毎回文面が変わって重複判定が効かなくなる。
    #[test]
    fn a_long_message_splits_into_context_and_question() {
        let body = "今日、父が河川敷でバーベキューをするそうです\n\
                    この前は皆で食事をしなかったから\n\
                    姉たちは帰ったのでいません\n\
                    くる？";
        let got = extract(body);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].text, "くる？");
        let context = got[0].context.as_ref().expect("状況説明が落ちている");
        assert!(context.contains("バーベキュー"));
        assert!(!context.contains("くる？"));
    }

    #[test]
    fn ignores_statements() {
        assert!(extract("明日行くね\nそのとき、郵便物も\n持参するね").is_empty());
        assert!(extract("月曜日の夜行きます。").is_empty());
    }

    #[test]
    fn picks_up_questions_without_a_question_mark() {
        assert_eq!(texts("いつ取得する予定ですか"), vec!["いつ取得する予定ですか"]);
        assert_eq!(texts("どうするつもりなのか"), vec!["どうするつもりなのか"]);
    }

    #[test]
    fn separates_multiple_questions_in_one_message() {
        let got = texts("免許証は、いつ取得する予定ですか？\nあと保険証はある？");
        assert_eq!(
            got,
            vec!["免許証は、いつ取得する予定ですか？", "あと保険証はある？"]
        );
    }

    // MARK: 分類

    #[test]
    fn visit_questions_are_recognised() {
        for q in [
            "明日来る？",
            "今日くる？",
            "こないの？",
            "泊まる？",
            "今日も来なかったね いつ来る？",
            "いつ来ますか？",
        ] {
            assert_eq!(classify(q), QuestionKind::Visit, "{q}");
        }
    }

    #[test]
    fn factual_questions_are_not_treated_as_visits() {
        for q in [
            "保険証は、ありますか？",
            "免許証は、いつ取得する予定ですか？",
            "何故ですか？",
            "これ、迷惑メールだよね？",
        ] {
            assert_eq!(classify(q), QuestionKind::Fact, "{q}");
        }
    }

    /// 「行く」系を訪問と見なすと、相手自身の行動や物のやり取りを
    /// 誤って拾い、無関係な質問に定型回答を返してしまう。
    #[test]
    fn the_other_partys_own_actions_are_not_visits() {
        assert_eq!(classify("持って行けないでしょう？"), QuestionKind::Fact);
        assert_eq!(classify("そっちに送ろうか？"), QuestionKind::Fact);
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
