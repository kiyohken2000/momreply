//! プロンプトの組み立て（仕様書 8）。
//!
//! 仕様書 8.1 のシステムプロンプトを土台にしつつ、2 点変えている。
//!
//! 1. **質問に具体的な答えを出させない。**
//!    仕様書は「答えるための材料」を用意して答えさせる前提だったが、
//!    それは材料が足りないたびに人へ確認が飛ぶということでもある。
//!    このアプリの目的は放置できることなので、確定させない返し方に倒す。
//!    誤った事実を自動送信する危険も、言い切らないぶん小さくなる。
//!
//! 2. **文体と内容の出どころを分ける。**
//!    few-shot は話し方の手本であって、書く内容の根拠ではない。
//!    過去に短く突き放していれば few-shot はそれを再生産する。
//!
//! # `self.md` の位置づけ
//!
//! 事実の一覧ではなく、**書き方の方向性を指示する場所**として扱う。
//! 「デスマス調にしない」のような指示を書けば、文例よりそちらが優先される。
//! 事実が書いてあれば、それはそのまま使ってよい材料になる。

use crate::{fewshot::Pair, llm::ChatMessage, questions::Question};

/// 生成に必要な材料。
pub struct Context {
    /// 相手の表示名。
    pub display_name: String,
    /// 自分の名前。プロンプトで「本人になりきる」ために使う。
    pub user_name: String,
    /// `self.md` 全文。**書き方の指示と、言い切ってよい事実。**
    pub self_profile: String,
    /// 相手プロファイル全文。
    pub target_profile: String,
    /// 文体の手本。
    pub fewshot: Vec<Pair>,
    /// 直近の会話（古い順）。`(相手か自分か, 本文)`。
    pub recent: Vec<(bool, String)>,
    /// 今回のメッセージ本文。
    pub incoming: String,
    /// 今回のメッセージから取り出した質問。
    pub questions: Vec<Question>,
    /// 現在日時の表示。
    pub now: String,
    /// 長さの指示（仕様書 6.9.3）。
    pub length_instruction: String,
    /// 再生成のときだけ入る（仕様書 8.3）。
    pub retry: Option<Retry>,
}

/// やり直しの指示。
#[derive(Debug, Clone)]
pub struct Retry {
    /// 前回の生成結果。
    pub previous: String,
    /// ユーザーの追加指示。空なら同じ条件でやり直す（仕様書 6.6）。
    pub instruction: Option<String>,
}

pub fn system(ctx: &Context) -> String {
    let mut s = String::new();

    s.push_str(&format!(
        "あなたは「{}」本人として、{}からの iMessage に返信を書きます。\n\
         アシスタントではなく、本人になりきってください。\n\n",
        ctx.user_name, ctx.display_name
    ));

    s.push_str(
        "# 絶対のルール\n\
         - 返信の本文のみを出力する。前置き・説明・引用符・コードブロックは一切つけない\n\
         - 「〜という返信はいかがでしょうか」のようなメタ発言は禁止\n\
         - 文例と同じ文体・語尾・絵文字の使い方を厳密に真似る\n\
         - 存在しない出来事をでっち上げない\n\
         - **相手のメッセージと同じ言語で書く。** この指示が日本語でも、\n\
           相手が英語なら英語で返す\n\n",
    );

    // ここがこのアプリの肝。仕様書 8.1 から意図的に変えている箇所。
    // 人に聞かずに済ませるための指示。断定させないことで、
    // 誤った事実を自動送信する危険を下げている。
    s.push_str(
        "# 答え方（重要）\n\
         質問されても、**具体的な答えを出さない。**\n\
         ただし、そっけなくしない。相手の話をちゃんと受け止めて、\n\
         たっぷり書く。読んで冷たく感じない文にすること。\n\
         \n\
         守ること:\n\
         - 日付・時刻・金額・可否を確定させない\n\
         - 約束をしない。「行く」「行かない」「やる」と言い切らない\n\
         - 断ることも承諾することもしない\n\
         - 『自分についてのメモ』に書いてあることだけは、そのまま言ってよい\n\
         - 書いていないことは作らない。知らないことは触れない\n\
         - 過去の会話は**状況を読むためだけ**に使う。そこに出てきた話を\n\
           「自分の考え」「自分の方針」として言い直さない\n\
         \n\
         やること:\n\
         - 相手が書いてきた話題に触れて、受け止めたと分かるようにする\n\
           （ただし内容の是非は判断しない）\n\
         - 近況めいた話、体調を気づかう言葉、当たり障りのない話を混ぜる\n\
         - 相手が続きを書きやすいように、軽い問いかけで終えてもよい\n\
         \n\
         人間に確認を求めてはいけない。\n\
         どんな内容でも、この方針で返信を書ききること。\n\n",
    );

    s.push_str(
        "# 文体と内容の使い分け（重要）\n\
         文例は「話し方」の手本であって、「何を書くか」の手本ではない。\n\
         語尾・句読点・絵文字の使い方だけを文例から借りること。\n\
         文例に短い返事が並んでいても、長さの指示のほうを優先する。\n\n",
    );

    s.push_str(&format!("# 返信の長さ\n{}\n", ctx.length_instruction));
    s.push_str(
        "※ ただし**長さより、作り話をしないことが優先**。\n\
         書くことが足りなくても、埋めるために出来事や状態をでっち上げない。\n\
         材料が足りなければ、指示より短くてよい。\n\
         長さの指示は文体より優先されるが、文体そのものを変えてはならない。\n\
         長く書く場合も、文例と同じくだけた話し言葉のまま書くこと。\n\n",
    );

    // 事実と指示が混ざって書かれる前提で読ませる。どちらなのかを
    // 人に仕分けさせると、結局「いちいち書く」手間に戻ってしまう。
    s.push_str(
        "# 自分についてのメモ\n\
         書き方の指示と、言い切ってよい事実が混ざって書かれている。\n\
         - 書き方の指示（例:「デスマス調にしない」）があれば、**文例より優先して従う**\n\
         - 事実が書いてあれば、それはそのまま言ってよい\n\
         - ここに無いことは作らない\n\n",
    );
    s.push_str(trimmed_or(&ctx.self_profile, "（未設定）"));
    s.push_str("\n\n");

    s.push_str(&format!("# {}について\n", ctx.display_name));
    s.push_str(trimmed_or(&ctx.target_profile, "（未設定）"));
    s.push_str("\n\n");

    s.push_str(&format!("# 現在\n{}\n\n", ctx.now));

    if !ctx.recent.is_empty() {
        s.push_str("# 直近の会話\n");
        for (from_me, body) in &ctx.recent {
            let who = if *from_me { "自分" } else { &ctx.display_name };
            s.push_str(&format!("{who}: {}\n", body.replace('\n', " ")));
        }
    }

    s
}

/// few-shot を会話形式で並べ、最後に今回のメッセージを置く（仕様書 8.2）。
///
/// system プロンプトにテキストとして埋めるより、実際の会話形式のほうが
/// 文体の模倣精度が高い。
pub fn messages(ctx: &Context) -> Vec<ChatMessage> {
    let mut out = Vec::with_capacity(ctx.fewshot.len() * 2 + 2);

    for p in &ctx.fewshot {
        out.push(ChatMessage::user(format!(
            "<{}> {}",
            ctx.display_name, p.incoming
        )));
        out.push(ChatMessage::assistant(&p.reply));
    }

    out.push(ChatMessage::user(final_turn(ctx)));

    // 再生成（仕様書 8.3）。前回の結果を assistant として置き、
    // その後ろにユーザーの指示を足す。
    if let Some(retry) = &ctx.retry {
        out.push(ChatMessage::assistant(&retry.previous));
        out.push(ChatMessage::user(retry_turn(retry)));
    }

    out
}

fn retry_turn(retry: &Retry) -> String {
    match retry.instruction.as_deref().map(str::trim) {
        Some(instruction) if !instruction.is_empty() => format!(
            "この返信を次の指示で書き直して: {instruction}\n本文のみ出力すること。"
        ),
        // 指示が無いときは同じ条件でやり直す。ただし前回と同じ文面を
        // 返されても意味が無いので、そこだけは伝える。
        _ => "この返信は使えなかった。前回とは別の内容で書き直して。\n本文のみ出力すること。"
            .to_string(),
    }
}

/// 最後の user メッセージ。
///
/// 方針と長さ指示をここで再掲する。**末尾の指示のほうが効きやすい**ため
/// （仕様書 6.9.4-3）。few-shot に短い返信が並ぶと、system 側の指示は
/// 押し負ける。
fn final_turn(ctx: &Context) -> String {
    let mut s = format!("<{}> {}", ctx.display_name, ctx.incoming);

    // 質問があることは伝えるが、答えさせない。
    if !ctx.questions.is_empty() {
        s.push_str("\n\n（質問が含まれているが、確定的な答えは出さないこと。");
        s.push_str("受け止めたことは伝えつつ、日付・可否・約束は避ける）");
    }

    s.push_str(&format!("\n\n（{}）", ctx.length_instruction));
    s
}

fn trimmed_or<'a>(s: &'a str, fallback: &'a str) -> &'a str {
    if s.trim().is_empty() {
        fallback
    } else {
        s.trim()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context {
        Context {
            display_name: "母".into(),
            user_name: "自分".into(),
            self_profile: "- デスマス調にしない\n- 保険証: 持っている".into(),
            target_profile: "## 基本\n- 呼び方: お母さん".into(),
            fewshot: vec![Pair {
                incoming: "ごはん食べた？".into(),
                reply: "食べたよー".into(),
                source_rowid: 1,
            }],
            recent: vec![(false, "元気？".into()), (true, "元気だよ".into())],
            incoming: "保険証はある？".into(),
            questions: vec![Question {
                text: "保険証はある？".into(),
                context: None,
            }],
            now: "2026年8月1日(土) 20:00".into(),
            length_instruction: "母のメッセージと同じくらいの長さで返す。".into(),
            retry: None,
        }
    }

    /// 放置できることが目的なので、人への確認を求めさせない。
    #[test]
    fn the_prompt_never_asks_the_human() {
        let s = system(&ctx());
        assert!(s.contains("人間に確認を求めてはいけない"));
    }

    /// 曖昧に返すのは、確定させないため。ここが緩むと意味が無い。
    #[test]
    fn the_prompt_forbids_commitments() {
        let s = system(&ctx());
        assert!(s.contains("具体的な答えを出さない"));
        assert!(s.contains("約束をしない"));
        assert!(s.contains("日付・時刻・金額・可否を確定させない"));
    }

    /// そっけない一言は、実データ上いちばん事態を悪くしていた形。
    #[test]
    fn the_prompt_requires_warmth_and_length() {
        let s = system(&ctx());
        assert!(s.contains("そっけなくしない"));
        assert!(s.contains("たっぷり書く"));
    }

    /// 曖昧でも、作り話をしてよいわけではない。
    #[test]
    fn the_prompt_forbids_invention() {
        let s = system(&ctx());
        assert!(s.contains("書いていないことは作らない"));
    }

    /// 指示が日本語であることに引きずられて日本語で返すと、
    /// 日本語以外のやり取りではまったく使えない。
    #[test]
    fn the_reply_follows_the_language_of_the_incoming_message() {
        let s = system(&ctx());
        assert!(s.contains("相手のメッセージと同じ言語で書く"));
        assert!(s.contains("相手が英語なら英語で返す"));
    }

    /// 長さを埋めるために出来事をでっち上げるのが、実運用で最初に出た事故。
    /// 「朝からあんまり動いてない」「水分はこまめにとってる」のような、
    /// どこにも書いていない近況が自動送信された。
    #[test]
    fn not_inventing_beats_the_length_instruction() {
        let s = system(&ctx());
        assert!(s.contains("長さより、作り話をしないことが優先"));
        assert!(s.contains("指示より短くてよい"));
    }

    /// 会話履歴から拾った話を「自分の方針」として言い直すのも同じ事故。
    /// 本人が言っていない立場が、本人の名前で表明される。
    #[test]
    fn the_history_is_context_not_a_position_to_restate() {
        let s = system(&ctx());
        assert!(s.contains("状況を読むためだけ"));
        assert!(s.contains("「自分の考え」「自分の方針」として言い直さない"));
    }

    /// few-shot が内容の根拠にされると、過去の突き放した返しを再生産する。
    #[test]
    fn the_prompt_separates_style_from_content() {
        let s = system(&ctx());
        assert!(s.contains("話し方"));
        assert!(s.contains("長さの指示のほうを優先する"));
    }

    /// self.md は事実の置き場であると同時に、書き方の指示の置き場である。
    /// 指示が文例に負けると、「デスマス調にしない」と書いても効かない。
    #[test]
    fn the_self_note_overrides_the_style_examples() {
        let s = system(&ctx());
        assert!(s.contains("書き方の指示"));
        assert!(s.contains("文例より優先して従う"));
        assert!(s.contains("デスマス調にしない"));
    }

    #[test]
    fn self_profile_is_included() {
        assert!(system(&ctx()).contains("保険証: 持っている"));
    }

    #[test]
    fn missing_profiles_do_not_leave_the_section_blank() {
        let mut c = ctx();
        c.self_profile = "   ".into();
        c.target_profile = String::new();
        assert_eq!(system(&c).matches("（未設定）").count(), 2);
    }

    #[test]
    fn fewshot_becomes_alternating_turns() {
        let msgs = messages(&ctx());
        assert_eq!(msgs.len(), 3); // 1 ペア + 今回
        assert_eq!(msgs[0].role, "user");
        assert!(msgs[0].content.starts_with("<母> ごはん食べた？"));
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content, "食べたよー");
        assert_eq!(msgs[2].role, "user");
    }

    /// 末尾に再掲しないと few-shot の短さに押し負ける（仕様書 6.9.4-3）。
    #[test]
    fn the_last_turn_repeats_the_stance_and_length() {
        let last = final_turn(&ctx());
        assert!(last.contains("保険証はある？"));
        assert!(last.contains("確定的な答えは出さないこと"));
        assert!(last.contains("同じくらいの長さ"));
    }

    #[test]
    fn a_message_without_questions_has_no_question_block() {
        let mut c = ctx();
        c.questions.clear();
        assert!(!final_turn(&c).contains("確定的な答えは出さないこと"));
    }

    // MARK: 再生成（仕様書 8.3）

    #[test]
    fn a_retry_appends_the_previous_result_and_the_instruction() {
        let mut c = ctx();
        c.retry = Some(Retry {
            previous: "作らない".into(),
            instruction: Some("来ないでほしいと伝えて".into()),
        });
        let msgs = messages(&c);
        let n = msgs.len();

        // 前回の結果が assistant として入り、その後ろに指示が来る。
        assert_eq!(msgs[n - 2].role, "assistant");
        assert_eq!(msgs[n - 2].content, "作らない");
        assert_eq!(msgs[n - 1].role, "user");
        assert!(msgs[n - 1].content.contains("来ないでほしいと伝えて"));
        assert!(msgs[n - 1].content.contains("本文のみ"));
    }

    /// 指示なしの再生成でも、前回と同じ文面を返されては意味が無い。
    #[test]
    fn a_retry_without_an_instruction_still_asks_for_something_different() {
        let turn = retry_turn(&Retry {
            previous: "作らない".into(),
            instruction: None,
        });
        assert!(turn.contains("別の内容"));
        assert!(turn.contains("本文のみ"));

        // 空白だけの指示も「指示なし」として扱う。
        let blank = retry_turn(&Retry {
            previous: "作らない".into(),
            instruction: Some("   ".into()),
        });
        assert_eq!(turn, blank);
    }

    #[test]
    fn without_a_retry_the_conversation_ends_with_the_incoming_message() {
        let msgs = messages(&ctx());
        assert_eq!(msgs.last().unwrap().role, "user");
        assert!(msgs.last().unwrap().content.contains("保険証はある？"));
    }
}
