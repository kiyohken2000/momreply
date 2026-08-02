//! 返信案を 1 件生成する。
//!
//! Phase 1 の範囲。送信はしない。結果は `dry_run` として記録する。

use anyhow::{bail, Context as _, Result};
use rusqlite::Connection;

use crate::{
    imessage,
    llm::{self, CompletionRequest, LlmError, Provider},
    pipeline::{clean, guards, prompt, Cleaned},
    profile, questions,
    store::{Store, Target},
};

/// 目標文字数の下限・上限。
///
/// 極端な値を入れられると、暴走検知の閾値ごと壊れる。UI 側でも弾くが、
/// 保存済みの値がここを通るので、読み出し側でも必ず丸める。
pub const MIN_TARGET_CHARS: u32 = 10;
pub const MAX_TARGET_CHARS: u32 = 2000;

/// 長さの指定（仕様書 6.9.1）。
///
/// プリセットは「だいたいこのくらい」を一発で選ぶためのもの。
/// [`LengthPreset::Chars`] は目標文字数を数値で指定したときの形で、
/// 選ばれていればプリセットより優先される。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthPreset {
    Mirror,
    Short,
    Normal,
    Long,
    VeryLong,
    /// 目標文字数を直接指定する。
    Chars(u32),
}

impl LengthPreset {
    /// `targets.reply_preset` の文字列から読む。
    ///
    /// 数値指定は `chars:400` の形で入る。列を増やさずに済み、
    /// プリセットと同じ 1 つの設定として扱える。
    pub fn parse(s: &str) -> Option<Self> {
        if let Some(n) = s.strip_prefix("chars:") {
            let n: u32 = n.trim().parse().ok()?;
            return Some(Self::Chars(n.clamp(MIN_TARGET_CHARS, MAX_TARGET_CHARS)));
        }
        match s {
            "mirror" => Some(Self::Mirror),
            "short" => Some(Self::Short),
            "normal" => Some(Self::Normal),
            "long" => Some(Self::Long),
            "very_long" => Some(Self::VeryLong),
            _ => None,
        }
    }

    /// 暴走検知の閾値。目標値ではない（仕様書 6.9.1）。
    pub fn hard_max_length(self) -> usize {
        match self {
            Self::Mirror => 300,
            Self::Short => 150,
            Self::Normal => 300,
            Self::Long => 800,
            Self::VeryLong => 2000,
            // 目標ちょうどには収まらない。閾値は暴走の検知が目的なので、
            // 狭く取ると普通の生成が確認送りになって手間が増える。
            Self::Chars(n) => (n as usize * 2).max(200),
        }
    }

    /// 生成の上限トークン。日本語 1 文字がおよそ 1 トークン強なので
    /// 閾値より広く取る。狭いと途中で切れる。
    pub fn max_tokens(self) -> u32 {
        (self.hard_max_length() as u32) * 3 + 256
    }

    pub fn instruction(self) -> String {
        match self {
            Self::Mirror => {
                "相手のメッセージと同じくらいの長さで返す。相手が一言なら一言で返す。".into()
            }
            Self::Short => "10〜40文字程度。1文で簡潔に。".into(),
            Self::Normal => "30〜100文字程度。1〜2文。".into(),
            Self::Long => "200〜400文字程度。近況や感想を具体的に添えて、3〜5文程度で書く。\
                 ただし文体は文例のまま崩さないこと。"
                .into(),
            Self::VeryLong => "600〜1200文字程度。近況、感想、質問などを織り交ぜてたっぷり書く。\
                 改行を使って読みやすくする。ただし文体・語尾・絵文字の使い方は\
                 文例のまま崩さないこと。丁寧語やビジネス文体に寄せてはいけない。"
                .into(),
            // 幅を持たせないと、字数合わせのために不自然な言い回しが増える。
            Self::Chars(n) => {
                let low = (n as f32 * 0.8).round() as u32;
                let high = (n as f32 * 1.2).round() as u32;
                format!(
                    "{low}〜{high}文字程度（目安 {n} 文字）で書く。\
                     文字数に合わせるために、丁寧語やビジネス文体へ寄せてはいけない。\
                     文体・語尾・絵文字の使い方は文例のまま崩さないこと。"
                )
            }
        }
    }
}

/// プロンプトに入れる会話。
///
/// **UI もこれを表示する。** 見せているものと渡したものがずれると、
/// なぜその返信になったのか説明できなくなる。だから組み立ては 1 か所。
pub struct Conversation {
    /// 返信の対象。連投ならまとめた分すべて（古い順）。
    pub burst: Vec<imessage::Message>,
    /// その前の会話（古い順）。本文のあるものだけ。
    pub recent: Vec<imessage::Message>,
}

/// 「直近の会話」として渡す件数。
const RECENT_TURNS: u32 = 20;

/// 返信対象の周辺を組み立てる。
pub fn conversation(
    chat_db: &Connection,
    handles: &[String],
    message: &imessage::Message,
) -> Result<Conversation> {
    // 相手が数行に分けて送ってきた分を 1 通にまとめる。
    //
    // 最後の 1 行だけを見ると中身が無いことがある。「返信なければ行く」
    // だけが残り、実際の問いは前の行にある、という形が実データに出る。
    let burst = imessage::burst(chat_db, handles, message, imessage::BURST_WINDOW)?;

    // まとめた分を履歴にも出すと、同じ文が 2 回入って重みが狂う。
    let in_burst: Vec<i64> = burst.iter().map(|m| m.rowid).collect();
    let recent = imessage::recent_messages(chat_db, handles, RECENT_TURNS)?
        .into_iter()
        .filter(|m| m.skip.is_none() && m.body.is_some() && !in_burst.contains(&m.rowid))
        .collect();

    Ok(Conversation { burst, recent })
}

pub struct Draft {
    pub chat_rowid: i64,
    pub incoming: String,
    pub text: String,
    pub provider: String,
    pub model: String,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub latency_ms: u64,
    /// 上限超過で確認に倒れたか（仕様書 6.2.1-5）。
    pub held_for_review: bool,
    /// ガードで止まった場合の理由（仕様書 6.4）。生成していない。
    pub skipped: Option<guards::SkipReason>,
}

/// 呼び出しの性質。リトライの粘り方が変わる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    /// 人が画面の前で待っている。**粘らない。**
    ///
    /// レート制限は数秒では明けない。待たせたうえで失敗するくらいなら、
    /// すぐ理由を見せて別の手段（プロバイダの切り替え）を選ばせるほうがよい。
    Interactive,
    /// 裏で動いている。誰も待っていないので粘ってよい。
    Background,
}

/// 再生成の指定（仕様書 6.6 / 8.3）。
///
/// 意味の通らない返信案が出たときに、人が指示を足してやり直させる。
/// 指示が `None` でも、前回と同じ文面を返させないようには伝える。
pub struct Redo<'a> {
    pub instruction: Option<&'a str>,
}

/// 直近の受信メッセージ 1 件に対して返信案を作る。
///
/// `message` は生成対象として選ばれた受信メッセージ。
pub async fn draft_reply(
    chat_db: &Connection,
    store: &Store,
    target: &Target,
    message: &imessage::Message,
    preset: LengthPreset,
    redo: Option<Redo<'_>>,
    urgency: Urgency,
) -> Result<Draft> {
    let incoming = message
        .body
        .clone()
        .context("本文が無いメッセージは生成対象にならない")?;

    // 生成の直前に既返信チェック（仕様書 6.4.3）。
    // 送信の直前にもう一度行う。生成に数秒かかる間に手で返信されうるため。
    let own_replies =
        imessage::count_own_replies_after(chat_db, &target.handles, message.rowid)?;
    if own_replies > 0 {
        return Ok(Draft {
            chat_rowid: message.rowid,
            incoming,
            text: String::new(),
            provider: String::new(),
            model: String::new(),
            input_tokens: None,
            output_tokens: None,
            latency_ms: 0,
            held_for_review: false,
            skipped: Some(guards::SkipReason::AlreadyReplied),
        });
    }

    let convo = conversation(chat_db, &target.handles, message)?;
    let incoming = if convo.burst.len() > 1 {
        imessage::burst_text(&convo.burst)
    } else {
        incoming
    };

    // 質問は「質問が来ている」と伝えるためだけに取り出す。
    // 答えさせないので、答えられるかどうかの判定はしない。
    let found = questions::extract(&incoming);

    let provider = primary_provider(store)?;
    let model = store
        .get_kv(&provider.model_setting_key())?
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| provider.default_model().to_string());


    // 再生成なら前回の結果を引く。無ければ通常生成に落とす。
    let retry = match &redo {
        Some(r) => store.previous_draft(message.rowid)?.map(|previous| prompt::Retry {
            previous,
            instruction: r.instruction.map(str::to_string),
        }),
        None => None,
    };
    let kind = if retry.is_some() { "regenerate" } else { "initial" };

    let ctx = prompt::Context {
        display_name: target.display_name.clone(),
        user_name: store
            .get_kv("user_name")?
            .unwrap_or_else(|| "自分".to_string()),
        self_profile: profile::read_self()?,
        target_profile: profile::read_target(&target.slug, &target.display_name)?,
        fewshot: store.fewshot(target.id)?,
        recent: convo
            .recent
            .iter()
            .filter_map(|m| m.body.clone().map(|b| (m.is_from_me, b)))
            .collect(),
        incoming: incoming.clone(),
        questions: found,
        now: chrono::Local::now().format("%Y年%-m月%-d日(%a) %H:%M").to_string(),
        length_instruction: preset.instruction(),
        retry,
    };

    let llm = llm::build(provider, Some(model.clone())).map_err(anyhow::Error::from)?;
    let response = call_with_retry(
        llm.as_ref(),
        urgency,
        CompletionRequest {
            model: model.clone(),
            system: prompt::system(&ctx),
            messages: prompt::messages(&ctx),
            max_tokens: preset.max_tokens(),
            temperature: 0.8,
        },
    )
    .await?;

    store.log_generation(&crate::store::GenerationRecord {
        target_id: target.id,
        chat_rowid: message.rowid,
        kind,
        provider: provider.id(),
        model: &model,
        input_tokens: response.input_tokens,
        output_tokens: response.output_tokens,
        latency_ms: response.latency_ms,
        user_instruction: redo.as_ref().and_then(|r| r.instruction),
        output: Some(&response.text),
        error: None,
    })?;

    let (text, held) = match clean(&response.text, preset.hard_max_length()) {
        Cleaned::Ok(t) => (t, false),
        Cleaned::TooLong { text, chars } => {
            // 送らずに確認へ倒す。長さの暴走は事故に直結する。
            eprintln!("警告: 生成が {chars} 文字で上限を超えた。送信せず確認に回す");
            (text, true)
        }
        Cleaned::Empty => bail!("生成結果が空だった"),
    };

    Ok(Draft {
        chat_rowid: message.rowid,
        incoming,
        text,
        provider: provider.id().to_string(),
        model,
        input_tokens: response.input_tokens,
        output_tokens: response.output_tokens,
        latency_ms: response.latency_ms,
        held_for_review: held,
        skipped: None,
    })
}

/// 指数バックオフで最大 3 回（仕様書 6.2）。
///
/// # 待ち時間を種類で変える
///
/// レート制限は**1 分あたり**で課されることが多い。500ms から始めると
/// 3 回目まで含めても 2 秒に満たず、制限が明ける前に諦めることになる。
/// 実際に Gemini でそうなった。ネットワークの一時エラーとは別扱いにする。
async fn call_with_retry(
    llm: &dyn llm::LlmProvider,
    urgency: Urgency,
    req: CompletionRequest,
) -> Result<llm::CompletionResponse> {
    const NETWORK_BASE: std::time::Duration = std::time::Duration::from_millis(500);
    const RATE_LIMIT_BASE: std::time::Duration = std::time::Duration::from_secs(20);

    let mut last: Option<LlmError> = None;

    for attempt in 0..3u32 {
        match llm.complete(req.clone()).await {
            // 画面の前で待っている人を、明けない制限のために止めない。
            Err(LlmError::RateLimit(body)) if urgency == Urgency::Interactive => {
                return Err(anyhow::anyhow!(
                    "レート制限に達しました。しばらく待つか、設定タブで別のAIに\
                     切り替えてください。{}",
                    brief_body(&body)
                ));
            }
            Ok(r) => return Ok(r),
            Err(e) if e.is_retryable() => {
                let base = if matches!(e, LlmError::RateLimit(_)) {
                    RATE_LIMIT_BASE
                } else {
                    NETWORK_BASE
                };
                let delay = base * 2u32.pow(attempt);
                eprintln!(
                    "{}回目の呼び出しに失敗（{}秒後に再試行）: {e}",
                    attempt + 1,
                    delay.as_secs().max(1)
                );
                last = Some(e);
                tokio::time::sleep(delay).await;
            }
            // Auth / InvalidOutput はリトライしても同じ結果になる。
            Err(e) => return Err(e.into()),
        }
    }
    Err(last
        .map(anyhow::Error::from)
        .unwrap_or_else(|| anyhow::anyhow!("生成に失敗した")))
}

/// API の応答を、画面に出せる長さに縮める。
fn brief_body(body: &str) -> String {
    let head: String = body.chars().filter(|c| *c != '\n').take(120).collect();
    if head.trim().is_empty() {
        String::new()
    } else {
        format!("（{head}）")
    }
}

/// 主プロバイダ。未設定なら設定済みのものから選ぶ。
fn primary_provider(store: &Store) -> Result<Provider> {
    if let Some(id) = store.get_kv("llm.primary")? {
        if let Some(p) = Provider::parse(&id) {
            return Ok(p);
        }
    }
    Provider::with_keys()
        .into_iter()
        .find(|p| llm::credentials::is_configured(*p))
        .context("APIキーが設定されたプロバイダがありません")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 閾値は目標値の 2 倍程度、という仕様書 6.9.1 の目安に沿っていること。
    #[test]
    fn presets_have_sane_limits() {
        assert!(LengthPreset::Short.hard_max_length() < LengthPreset::Normal.hard_max_length());
        assert!(LengthPreset::Normal.hard_max_length() < LengthPreset::Long.hard_max_length());
        assert!(LengthPreset::Long.hard_max_length() < LengthPreset::VeryLong.hard_max_length());
    }

    /// トークン枠が閾値より狭いと、上限に届く前に生成が切れて
    /// 「短いのに未完成」という最悪の出力になる。
    #[test]
    fn token_budget_exceeds_the_character_limit() {
        for p in [
            LengthPreset::Mirror,
            LengthPreset::Short,
            LengthPreset::Normal,
            LengthPreset::Long,
            LengthPreset::VeryLong,
            LengthPreset::Chars(MIN_TARGET_CHARS),
            LengthPreset::Chars(400),
            LengthPreset::Chars(MAX_TARGET_CHARS),
        ] {
            assert!(
                p.max_tokens() as usize > p.hard_max_length(),
                "{p:?} のトークン枠が狭すぎる"
            );
        }
    }

    // MARK: 目標文字数

    #[test]
    fn a_character_target_round_trips() {
        assert_eq!(LengthPreset::parse("chars:400"), Some(LengthPreset::Chars(400)));
        assert_eq!(LengthPreset::parse("chars: 400 "), Some(LengthPreset::Chars(400)));
    }

    /// 極端な値で暴走検知の閾値ごと壊れないこと。
    /// UI でも弾くが、保存済みの値がここを通る。
    #[test]
    fn a_character_target_is_clamped() {
        assert_eq!(LengthPreset::parse("chars:0"), Some(LengthPreset::Chars(MIN_TARGET_CHARS)));
        assert_eq!(
            LengthPreset::parse("chars:999999"),
            Some(LengthPreset::Chars(MAX_TARGET_CHARS))
        );
    }

    #[test]
    fn a_broken_character_target_is_rejected() {
        assert_eq!(LengthPreset::parse("chars:"), None);
        assert_eq!(LengthPreset::parse("chars:abc"), None);
        assert_eq!(LengthPreset::parse("chars:-5"), None);
    }

    /// 閾値が目標ちょうどだと、普通の生成まで確認送りになって手間が増える。
    #[test]
    fn the_limit_leaves_room_above_the_target() {
        assert!(LengthPreset::Chars(400).hard_max_length() > 400);
        // 短い目標でも、下限を割って極端に狭くならないこと。
        assert!(LengthPreset::Chars(MIN_TARGET_CHARS).hard_max_length() >= 200);
    }

    /// 字数合わせのために文体が崩れるのが、実データ上いちばん多い失敗。
    #[test]
    fn a_character_target_still_insists_on_keeping_the_voice() {
        let i = LengthPreset::Chars(400).instruction();
        assert!(i.contains("400"));
        assert!(i.contains("丁寧語"));
        assert!(i.contains("文体"));
    }

    #[test]
    fn long_presets_insist_on_keeping_the_voice() {
        // 長く書かせると丁寧語に寄る（仕様書 14.9）。指示に明記されていること。
        assert!(LengthPreset::Long.instruction().contains("文体"));
        assert!(LengthPreset::VeryLong.instruction().contains("丁寧語"));
    }

    #[test]
    fn preset_names_round_trip() {
        for (s, p) in [
            ("mirror", LengthPreset::Mirror),
            ("short", LengthPreset::Short),
            ("normal", LengthPreset::Normal),
            ("long", LengthPreset::Long),
            ("very_long", LengthPreset::VeryLong),
        ] {
            assert_eq!(LengthPreset::parse(s), Some(p));
        }
        assert_eq!(LengthPreset::parse("なにか"), None);
    }
}
