//! 通知（仕様書 6.5 / 6.2）。
//!
//! **本文の全文は通知に載せない。** 通知はロック画面にも出るため、
//! 会話の中身が第三者の目に触れうる。何が起きたかと、確認が要ることだけ
//! 伝えて、詳細はポップオーバーで見せる。

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::lang;

/// 通知に載せてよい長さ。これを超えたら切る。
const PREVIEW_CHARS: usize = 40;

fn preview(s: &str) -> String {
    let one_line = s.replace('\n', " ");
    let head: String = one_line.chars().take(PREVIEW_CHARS).collect();
    if one_line.chars().count() > PREVIEW_CHARS {
        format!("{head}…")
    } else {
        head
    }
}

fn show(app: &AppHandle, title: &str, body: &str) {
    if let Err(why) = app.notification().builder().title(title).body(body).show() {
        // 通知が出せなくても本処理は続ける。
        eprintln!("警告: 通知を出せない: {why}");
    }
}

/// 生成はしたが、確認してから送る必要があるとき。
pub fn awaiting_review(app: &AppHandle, who: &str, reason: &str, draft: &str) {
    let l = lang::current();
    let title = match l {
        lang::Lang::Ja => format!("{who} への返信案を確認してください"),
        lang::Lang::En => format!("Review the draft reply to {who}"),
    };
    show(app, &title, &format!("{}｜{}", preview(draft), reason));
}

/// 送信したとき（仕様書 6.5「送信後は必ず通知を出す」）。
pub fn sent(app: &AppHandle, who: &str, text: &str) {
    let title = match lang::current() {
        lang::Lang::Ja => format!("{who} に返信しました"),
        lang::Lang::En => format!("Replied to {who}"),
    };
    show(app, &title, &preview(text));
}

/// 送ったかどうか分からないとき。**再送はしていない。**
pub fn send_unverified(app: &AppHandle, who: &str) {
    let l = lang::current();
    let title = match l {
        lang::Lang::Ja => format!("{who} への送信を確認できませんでした"),
        lang::Lang::En => format!("Could not verify the message to {who}"),
    };
    show(
        app,
        &title,
        l.pick(
            "メッセージ.app で実際の状態を確認してください。再送はしていません。",
            "Check Messages.app for the actual state. Nothing was resent.",
        ),
    );
}

pub fn failed(app: &AppHandle, who: &str, reason: &str) {
    let title = match lang::current() {
        lang::Lang::Ja => format!("{who} への返信に失敗しました"),
        lang::Lang::En => format!("Failed to reply to {who}"),
    };
    show(app, &title, &preview(reason));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 通知はロック画面にも出る。全文を載せない。
    #[test]
    fn long_text_is_truncated() {
        let long = "あ".repeat(200);
        let out = preview(&long);
        assert!(out.chars().count() <= PREVIEW_CHARS + 1);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn short_text_is_untouched() {
        assert_eq!(preview("わかった"), "わかった");
    }

    /// 改行が入ると通知の表示が崩れる。
    #[test]
    fn newlines_are_flattened() {
        assert_eq!(preview("明日は\n行かない"), "明日は 行かない");
    }
}
