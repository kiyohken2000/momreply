//! プロンプトの組み立て（仕様書 8）。
//!
//! 仕様書 8.1 のシステムプロンプトを土台にしつつ、2 点変えている。
//!
//! 1. **「確信のない事実は曖昧に返す」を、そのままにしない。**
//!    仕様書は「曖昧なら『確認してみる』と返す」としているが、
//!    質問に答えることが目的のこのアプリでは、はぐらかしは失敗である。
//!    答える材料が無い質問はそもそも生成に回さず人間に聞く
//!    （[`crate::questions`] と `pending_questions`）。ここまで来た質問は
//!    材料があるものなので、**はっきり答えさせる。**
//!
//! 2. **文体と内容の出どころを分ける。**
//!    few-shot は話し方の手本であって、答えの根拠ではない。
//!    過去に短く突き放していれば few-shot はそれを再生産する。
//!    何を答えるかは `self.md` と定型回答から取る、と明示する。

use crate::{fewshot::Pair, llm::ChatMessage, questions::Question};

/// 生成に必要な材料。
pub struct Context {
    /// 相手の表示名。
    pub display_name: String,
    /// 自分の名前。プロンプトで「本人になりきる」ために使う。
    pub user_name: String,
    /// `self.md` 全文。**AI が断定してよい唯一の材料。**
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
    /// 質問に対して既に分かっている答え（`self.md` 由来・定型回答）。
    pub known_answers: Vec<(String, String)>,
    /// 現在日時の表示。
    pub now: String,
    /// 長さの指示（仕様書 6.9.3）。
    pub length_instruction: String,
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
         - 存在しない出来事をでっち上げない\n\n",
    );

    // ここがこのアプリの肝。仕様書 8.1 から意図的に変えている箇所。
    s.push_str(
        "# 質問には必ず答える\n\
         相手の質問をはぐらかさない。「確認してみる」「また連絡する」で\n\
         済ませない。答えは下の『答えるための材料』に書いてある。\n\
         材料に書いてあることは、迷わず言い切る。\n\
         材料に無いことは推測しない。その場合だけ、分からないと正直に書く。\n\n",
    );

    s.push_str(
        "# 文体と内容の使い分け（重要）\n\
         文例は「話し方」の手本であって、「何を答えるか」の手本ではない。\n\
         語尾・句読点・絵文字の使い方だけを文例から借りること。\n\
         答えの中身は必ず『答えるための材料』から取る。\n\
         文例に短い返事が並んでいても、質問に答える必要があるなら答える。\n\n",
    );

    s.push_str(&format!("# 返信の長さ\n{}\n", ctx.length_instruction));
    s.push_str(
        "※ 長さの指示は文体より優先されるが、文体そのものを変えてはならない。\n\
         長く書く場合も、文例と同じくだけた話し言葉のまま書くこと。\n\n",
    );

    s.push_str("# 答えるための材料（自分について）\n");
    s.push_str(trimmed_or(&ctx.self_profile, "（未設定）"));
    s.push_str("\n\n");

    if !ctx.known_answers.is_empty() {
        s.push_str("# 今回の質問に対する答え\n");
        s.push_str("以下はあなた本人が確認済みの内容。これをそのまま使うこと。\n");
        for (q, a) in &ctx.known_answers {
            s.push_str(&format!("- 「{q}」→ {a}\n"));
        }
        s.push('\n');
    }

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
    out
}

/// 最後の user メッセージ。
///
/// 質問と長さ指示をここで再掲する。**末尾の指示のほうが効きやすい**ため
/// （仕様書 6.9.4-3）。few-shot に短い返信が並ぶと、system 側の指示は
/// 押し負ける。
fn final_turn(ctx: &Context) -> String {
    let mut s = format!("<{}> {}", ctx.display_name, ctx.incoming);

    if !ctx.questions.is_empty() {
        s.push_str("\n\n（この中の質問に必ず答えること:");
        for q in &ctx.questions {
            s.push_str(&format!("\n・{}", q.text));
        }
        s.push(')');
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
            self_profile: "## 事実\n- 保険証: 持っている".into(),
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
            known_answers: vec![("保険証はある？".into(), "持っている".into())],
            now: "2026年8月1日(土) 20:00".into(),
            length_instruction: "母のメッセージと同じくらいの長さで返す。".into(),
        }
    }

    /// はぐらかしを許すと、質問に答えるという目的が達成できない。
    #[test]
    fn the_prompt_forbids_deflection() {
        let s = system(&ctx());
        assert!(s.contains("はぐらかさない"));
        assert!(s.contains("確認してみる"));
        assert!(s.contains("言い切る"));
    }

    /// few-shot が答えの根拠にされると、過去の突き放した返しを再生産する。
    #[test]
    fn the_prompt_separates_style_from_content() {
        let s = system(&ctx());
        assert!(s.contains("話し方"));
        assert!(s.contains("答えの中身は必ず『答えるための材料』から取る"));
    }

    #[test]
    fn known_answers_are_shown_verbatim() {
        let s = system(&ctx());
        assert!(s.contains("「保険証はある？」→ 持っている"));
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
    fn the_last_turn_repeats_the_questions_and_length() {
        let last = final_turn(&ctx());
        assert!(last.contains("保険証はある？"));
        assert!(last.contains("必ず答えること"));
        assert!(last.contains("同じくらいの長さ"));
    }

    #[test]
    fn a_message_without_questions_has_no_question_block() {
        let mut c = ctx();
        c.questions.clear();
        assert!(!final_turn(&c).contains("必ず答えること"));
    }
}
