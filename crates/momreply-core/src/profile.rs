//! プロファイルの読み書き。
//!
//! 2 種類ある。
//!
//! - **相手プロファイル**（`targets/<slug>.md`）— 仕様書 5.3。相手について。
//! - **自分プロファイル**（`self.md`）— 仕様書には無い。**自分について**。
//!
//! `self.md` は 2 つの役割を持つ。
//!
//! 1. **文章の方向性の指示。** 「デスマス調にしない」のような、生成の
//!    仕方そのものへの指示。文体の手本より優先される。
//! 2. **言い切ってよい事実。** 返信は基本的に何も確定させないが、
//!    ここに書いてあることだけは、そのまま言ってよい。
//!
//! 相手プロファイルをいくら充実させても、自分側のことは出てこない。
//! 答えも書き方の好みも、持っているのは本人だけである。

use std::path::Path;

use anyhow::{Context, Result};

use crate::paths;

/// `self.md` の初期テンプレート。
pub const SELF_TEMPLATE: &str = r#"# 自分について

## 書き方
<!-- 文章の方向性への指示。文体の手本より優先される。1行1件。
     例: デスマス調にしない / 絵文字は使わない / 謝りすぎない -->

## 事実
<!-- ここに書いたことだけは、そのまま言ってよい。1行1件。 -->

## 答えたくないこと
<!-- 聞かれても触れない話題。 -->
"#;

/// 相手プロファイルの初期テンプレート（仕様書 5.3）。
pub const TARGET_TEMPLATE: &str = r#"# {display_name} のプロファイル

## 基本
- 呼び方:
- 自分の呼ばれ方:
- 居住地:

## 家族・人間関係

## 健康・通院

## 予定・イベント

## 会話のクセ

## 触れないほうがいいこと
"#;

/// `self.md` を読む。無ければテンプレートを作ってから読む。
pub fn read_self() -> Result<String> {
    let path = paths::self_profile()?;
    ensure_file(&path, SELF_TEMPLATE)?;
    std::fs::read_to_string(&path)
        .with_context(|| format!("self.md を読めない: {}", path.display()))
}

/// 相手プロファイルを読む。無ければテンプレートを作ってから読む。
pub fn read_target(slug: &str, display_name: &str) -> Result<String> {
    let path = paths::target_profile(slug)?;
    ensure_file(&path, &TARGET_TEMPLATE.replace("{display_name}", display_name))?;
    std::fs::read_to_string(&path)
        .with_context(|| format!("プロファイルを読めない: {}", path.display()))
}

/// 確認した事実を `self.md` の「## 事実」に 1 行追記する。
///
/// 未回答質問に答えたときに呼ぶ。これによって同じ質問が二度目に来たときは
/// 人間に聞かずに済む。
pub fn append_fact(question: &str, answer: &str) -> Result<()> {
    let path = paths::self_profile()?;
    ensure_file(&path, SELF_TEMPLATE)?;
    let current = std::fs::read_to_string(&path)?;
    let line = format!("- {}: {}", question.trim(), answer.trim());
    let updated = insert_under_heading(&current, "## 事実", &line);
    std::fs::write(&path, updated)
        .with_context(|| format!("self.md に書けない: {}", path.display()))?;
    Ok(())
}

/// 承認された候補を `self.md` の指定見出しに追記する。
///
/// 既存の記述には触れない。人が手で書いた内容を壊さないため。
pub fn append_to_section(section: &str, line: &str) -> Result<()> {
    let path = paths::self_profile()?;
    ensure_file(&path, SELF_TEMPLATE)?;
    let current = std::fs::read_to_string(&path)?;
    let heading = format!("## {}", section.trim_start_matches("## "));
    let entry = format!("- {}", line.trim().trim_start_matches("- "));
    let updated = insert_under_heading(&current, &heading, &entry);
    std::fs::write(&path, updated)
        .with_context(|| format!("self.md に書けない: {}", path.display()))?;
    Ok(())
}

/// `self.md` を丸ごと置き換える。UI からの編集に使う。
pub fn write_self(content: &str) -> Result<()> {
    let path = paths::self_profile()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, content)
        .with_context(|| format!("self.md に書けない: {}", path.display()))?;
    Ok(())
}

fn ensure_file(path: &Path, template: &str) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, template)
        .with_context(|| format!("作成できない: {}", path.display()))?;
    Ok(())
}

/// 指定見出しのセクション末尾に 1 行足す。見出しが無ければ末尾に作る。
///
/// ユーザーが手で書いた内容を壊さないよう、既存行の並びには手を触れない。
fn insert_under_heading(content: &str, heading: &str, line: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();

    let Some(start) = lines.iter().position(|l| l.trim() == heading) else {
        let mut out = content.trim_end().to_string();
        out.push_str(&format!("\n\n{heading}\n{line}\n"));
        return out;
    };

    // 次の見出しの直前（末尾の空行は挟んだまま）に挿入する。
    let mut end = lines.len();
    for (i, l) in lines.iter().enumerate().skip(start + 1) {
        if l.starts_with("## ") || l.starts_with("# ") {
            end = i;
            break;
        }
    }
    while end > start + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }

    let mut out: Vec<String> = lines[..end].iter().map(|s| s.to_string()).collect();
    out.push(line.to_string());
    out.extend(lines[end..].iter().map(|s| s.to_string()));

    let mut joined = out.join("\n");
    if content.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_a_fact_under_the_right_heading() {
        let got = insert_under_heading(SELF_TEMPLATE, "## 事実", "- 保険証: 持っている");
        let facts_idx = got.find("## 事実").unwrap();
        let fact_idx = got.find("- 保険証: 持っている").unwrap();
        let next_idx = got.find("## 答えたくないこと").unwrap();
        assert!(facts_idx < fact_idx && fact_idx < next_idx);
    }

    #[test]
    fn keeps_existing_lines_untouched() {
        let original = "# 自分について\n\n## 事実\n- 既存の行\n\n## 伝え方\n- 言い切る\n";
        let got = insert_under_heading(original, "## 事実", "- 追加の行");
        assert!(got.contains("- 既存の行"));
        assert!(got.contains("- 言い切る"));
        assert!(got.find("- 既存の行").unwrap() < got.find("- 追加の行").unwrap());
        assert!(got.find("- 追加の行").unwrap() < got.find("## 伝え方").unwrap());
    }

    #[test]
    fn creates_the_heading_when_missing() {
        let got = insert_under_heading("# 自分について\n", "## 事実", "- 保険証: ある");
        assert!(got.contains("## 事実"));
        assert!(got.contains("- 保険証: ある"));
    }

    #[test]
    fn appending_twice_keeps_both() {
        let once = insert_under_heading(SELF_TEMPLATE, "## 事実", "- A: 1");
        let twice = insert_under_heading(&once, "## 事実", "- B: 2");
        assert!(twice.contains("- A: 1"));
        assert!(twice.contains("- B: 2"));
    }
}
