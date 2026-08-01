//! 過去のやり取りから `self.md` の追記候補を作る。
//!
//! # 何を材料にするか
//!
//! **「相手が質問し、自分が答えた」ペアだけ**を見る。会話全体を投げない。
//! 自分についての事実はそこにしか現れないうえ、全文を投げると
//! 費用も、外部に出るデータ量も跳ね上がる。
//!
//! # 承認を必須にする理由
//!
//! `self.md` は AI が事実として断定する唯一の材料である。誤りが 1 行でも
//! 入ると、以後すべての生成が汚染される。仕様書 6.7 が母プロファイルに
//! ついて「自動反映はしない」としているのと同じで、こちらはより影響が
//! 大きい。**このモジュールは候補を作るだけで、`self.md` には書かない。**

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Deserialize;

use crate::{
    fewshot,
    imessage,
    llm::{self, ChatMessage, CompletionRequest, Provider},
    questions,
    store::{FactCandidate, Store},
};

/// 1 回の呼び出しに載せるペア数。
///
/// 多いと出力が長くなり、上限で切れて JSON が壊れる。実際に 20 件・
/// 2000 トークンで切れたので、件数を減らし枠を広げた。
const BATCH: usize = 10;

/// 抽出の出力枠。
///
/// 推論モデルは思考に先に使うため、出力ぶんだけでは足りない。
/// 途中で切れると JSON が壊れてバッチ丸ごと無駄になる。
const MAX_TOKENS: u32 = 8000;

const SYSTEM: &str = r#"あなたは、ある人物の「自分についての事実」を、その人の返信から抽出します。

抽出の基準はただ一つ、**半年後に読んでもまだ正しいか**です。
その場の返事は、たとえ本人の発言でも事実ではありません。

# 抽出するもの
- 持ち物や契約の有無（例: 保険証を持っている / 車は持っていない）
- 変わりにくい方針（例: マイナンバーカードは作らない）
- 継続する状態（例: 実家には行かないことにしている）
- 聞かれても答えたくないと読み取れる話題

# 抽出しないもの（重要）
- **一度きりの予定への返事。**「行く」「行かない」「明日は無理」など。
  日付や特定の催しに紐づく返事はすべてこれにあたる
- **相手への指示や助言。**「入っておいて」「捨てていいよ」は相手の話であって
  この人の事実ではない
- **その時だけの依頼。**「押入れのキーボードを捨てて」など
- 今日の天気、今から出る、といったその場の状況
- 推測。返信から直接読み取れないことは出さない

# 判断に迷ったら出さない
誤った事実が 1 行入ると、以後の文面すべてに影響します。
確信が持てないものは confidence を low にするのではなく、**出さない**でください。

# 出力
JSON のみ。前置きも説明もつけない。該当が無ければ {"facts": []}。

{"facts": [
  {"section": "事実", "content": "保険証: 持っている", "source_index": 0, "confidence": "high"}
]}

- section は "事実" | "答えたくないこと" | "伝え方" のいずれか
- content は「項目: 内容」の形で 1 行。40 文字以内
- source_index は入力の番号
- confidence は high | medium | low。返信が短く解釈の幅があるものは low
"#;

#[derive(Deserialize)]
struct Extracted {
    #[serde(default)]
    facts: Vec<Fact>,
}

#[derive(Deserialize)]
struct Fact {
    section: String,
    content: String,
    #[serde(default)]
    source_index: usize,
    #[serde(default = "default_confidence")]
    confidence: String,
}

fn default_confidence() -> String {
    "medium".to_string()
}

/// 走査結果。
pub struct ScanReport {
    pub pairs_examined: usize,
    pub candidates_added: usize,
    pub batches: usize,
}

/// 指定した会話から候補を作る。
///
/// `handles` に渡した会話しか読まない。**全会話を既定にしない。**
/// 対象外の相手を読まないという原則（仕様書 6.4.1）を、
/// この機能でも呼び出し側が明示的に破る形にしておく。
pub async fn scan(
    chat_db: &Connection,
    store: &Store,
    handles: &[String],
    scan_messages: u32,
    max_pairs: usize,
) -> Result<ScanReport> {
    let messages = imessage::recent_messages(chat_db, handles, scan_messages)?;

    // 質問に答えたペアだけに絞る。事実はそこにしか無い。
    let pairs: Vec<fewshot::Pair> = fewshot::build_pairs(&messages)
        .into_iter()
        .filter(|p| !questions::extract(&p.incoming).is_empty())
        .collect();

    let pairs: Vec<fewshot::Pair> = if pairs.len() > max_pairs {
        pairs[pairs.len() - max_pairs..].to_vec()
    } else {
        pairs
    };

    let provider = primary(store)?;
    let model = store
        .get_kv(&provider.model_setting_key())?
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| provider.default_model().to_string());
    let llm = llm::build(provider, Some(model.clone())).map_err(anyhow::Error::from)?;

    let mut added = 0usize;
    let mut batches = 0usize;

    for chunk in pairs.chunks(BATCH) {
        batches += 1;
        let listing = chunk
            .iter()
            .enumerate()
            .map(|(i, p)| {
                format!(
                    "[{i}]\n聞かれたこと: {}\n自分の返信: {}",
                    p.incoming.replace('\n', " "),
                    p.reply.replace('\n', " ")
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let response = llm
            .complete(CompletionRequest {
                model: model.clone(),
                system: SYSTEM.to_string(),
                messages: vec![ChatMessage::user(listing)],
                max_tokens: MAX_TOKENS,
                // 事実の抽出なので揺らがせない。
                temperature: 0.0,
            })
            .await;

        let response = match response {
            Ok(r) => r,
            Err(why) => {
                eprintln!("警告: 一部の抽出に失敗（続行）: {why}");
                continue;
            }
        };

        let parsed = match parse(&response.text) {
            Some(p) => p,
            None => {
                // 何が返ってきたか分からないと直しようがない。
                let head: String = response.text.chars().take(200).collect();
                eprintln!("警告: JSON を解釈できないバッチをとばした。応答: {head:?}");
                continue;
            }
        };

        for fact in parsed.facts {
            let src = chunk.get(fact.source_index);
            let candidate = FactCandidate {
                id: 0,
                section: normalize_section(&fact.section),
                content: fact.content.trim().to_string(),
                evidence_ask: src.map(|p| p.incoming.clone()),
                evidence_reply: src.map(|p| p.reply.clone()),
                source_rowid: src.map(|p| p.source_rowid),
                source_chat: handles.first().cloned(),
                confidence: fact.confidence,
            };
            if candidate.content.is_empty() {
                continue;
            }
            if store.add_fact_candidate(&candidate)? {
                added += 1;
            }
        }
    }

    Ok(ScanReport {
        pairs_examined: pairs.len(),
        candidates_added: added,
        batches,
    })
}

/// 見出しを既知のものに寄せる。想定外の値は「事実」に落とす。
fn normalize_section(s: &str) -> String {
    match s.trim() {
        "答えたくないこと" => "答えたくないこと".to_string(),
        "伝え方" => "伝え方".to_string(),
        _ => "事実".to_string(),
    }
}

/// モデルが前後に文章を付けてくることがあるので、JSON 部分だけ取り出す。
fn parse(raw: &str) -> Option<Extracted> {
    if let Ok(v) = serde_json::from_str::<Extracted>(raw.trim()) {
        return Some(v);
    }
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<Extracted>(&raw[start..=end]).ok()
}

fn primary(store: &Store) -> Result<Provider> {
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

    #[test]
    fn plain_json_parses() {
        let v = parse(r#"{"facts":[{"section":"事実","content":"保険証: 持っている","source_index":0,"confidence":"high"}]}"#).unwrap();
        assert_eq!(v.facts.len(), 1);
        assert_eq!(v.facts[0].content, "保険証: 持っている");
    }

    /// モデルが前置きやコードブロックを付けても拾う。
    #[test]
    fn json_wrapped_in_prose_is_recovered() {
        let v = parse("以下が結果です:\n```json\n{\"facts\":[]}\n```\n以上").unwrap();
        assert!(v.facts.is_empty());
    }

    #[test]
    fn broken_output_is_not_a_panic() {
        assert!(parse("すみません、抽出できませんでした").is_none());
        assert!(parse("").is_none());
        assert!(parse("}{").is_none());
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let v = parse(r#"{"facts":[{"section":"事実","content":"車: 持っていない"}]}"#).unwrap();
        assert_eq!(v.facts[0].confidence, "medium");
        assert_eq!(v.facts[0].source_index, 0);
    }

    /// 見出しを勝手に増やされると self.md の構造が崩れる。
    #[test]
    fn unknown_sections_collapse_to_facts() {
        assert_eq!(normalize_section("事実"), "事実");
        assert_eq!(normalize_section("答えたくないこと"), "答えたくないこと");
        assert_eq!(normalize_section("伝え方"), "伝え方");
        assert_eq!(normalize_section("健康・通院"), "事実");
        assert_eq!(normalize_section(""), "事実");
    }
}
