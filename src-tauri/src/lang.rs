//! 表示言語。
//!
//! フロントだけに持たせると、画面は英語なのに通知とメニューバーの
//! ツールチップだけ日本語、という状態になる。同じ値を app.db の kv から
//! 読んで、Rust 側の文言もそろえる。
//!
//! **訳すのは画面と通知に出る文字だけ。** コメントとログは日本語のまま
//! にしてある。読む相手が違う。

use momreply_core::store::Store;

pub const KEY: &str = "ui_language";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ja,
    En,
}

impl Lang {
    pub fn parse(s: &str) -> Self {
        if s == "ja" { Lang::Ja } else { Lang::En }
    }

    pub fn id(self) -> &'static str {
        match self {
            Lang::Ja => "ja",
            Lang::En => "en",
        }
    }

    /// 言語ごとの文言を選ぶ。
    pub fn pick(self, ja: &'static str, en: &'static str) -> &'static str {
        match self {
            Lang::Ja => ja,
            Lang::En => en,
        }
    }
}

/// 保存された設定。**既定は日本語。**
///
/// 読めないときも日本語に倒す。ここで失敗しても通知は出したいので、
/// 呼び出し側にエラーを渡さない。
pub fn current() -> Lang {
    Store::open_default()
        .ok()
        .and_then(|s| s.get_kv(KEY).ok().flatten())
        .map(|v| Lang::parse(&v))
        .unwrap_or(Lang::Ja)
}
