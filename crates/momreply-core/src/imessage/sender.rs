//! AppleScript による送信と、その結果の検証（仕様書 6.3）。
//!
//! # なぜ検証が要るか
//!
//! **AppleScript は送信に失敗してもエラーを返さないことがある。**
//! `osascript` の終了コード 0 を信じてはいけない（仕様書 14.3）。
//! 送信したつもりで届いていない、という状態が最も困る。
//! 送信後に chat.db を見て、実際に自分の送信として記録されたかを確かめる。

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use rusqlite::Connection;

use super::reader;

/// スクリプトはバイナリに埋め込む。実行時にファイルを探しにいかない。
const SEND_SCRIPT: &str = include_str!("send.applescript");

/// 送信検証のポーリング間隔。
const VERIFY_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// 通常の検証タイムアウト。
pub const VERIFY_TIMEOUT: Duration = Duration::from_secs(30);
/// 初回のみ。Messages.app の起動を待つ（仕様書 6.3）。
pub const VERIFY_TIMEOUT_FIRST: Duration = Duration::from_secs(60);

/// 送信の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// chat.db で確認できた。
    Sent { rowid: i64 },
    /// osascript は成功したが chat.db に現れなかった。
    ///
    /// **自動リトライしないこと。** 実は届いていて記録が遅れているだけの
    /// 場合があり、再送すると二重送信になる（仕様書 6.3）。
    Unverified,
}

/// 送信する。**この関数は宛先を検査しない。**
/// 呼び出し側がガード（仕様書 6.4）を通してから呼ぶこと。
///
/// `chat_identifier` は**受信したメッセージと同じもの**を渡す（仕様書 6.3）。
/// 設定の handles 配列の先頭などを使ってはいけない。SMS で受けた話に
/// iMessage で返すと、相手の画面では会話が 2 本に割れて見える。
pub fn send(chat_identifier: &str, text: &str) -> Result<()> {
    if chat_identifier.trim().is_empty() {
        bail!("送信先が空");
    }
    if text.trim().is_empty() {
        bail!("本文が空のメッセージは送らない");
    }

    // スクリプトは標準入力から渡す。一時ファイルを作らないので
    // 本文がディスクに残らない。
    let mut child = Command::new("osascript")
        .arg("-") // スクリプトを stdin から読む
        .arg(chat_identifier)
        .arg(text)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("osascript を起動できない")?;

    child
        .stdin
        .as_mut()
        .context("osascript の stdin を開けない")?
        .write_all(SEND_SCRIPT.as_bytes())
        .context("AppleScript を渡せない")?;

    let output = child.wait_with_output().context("osascript の終了を待てない")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // よくある失敗はオートメーション権限の未許可。
        bail!(
            "osascript が失敗した（終了コード {:?}）: {}\n\
             システム設定 → プライバシーとセキュリティ → オートメーション で\n\
             メッセージ.app の操作が許可されているか確認すること。",
            output.status.code(),
            stderr.trim()
        );
    }

    Ok(())
}

/// 送信が chat.db に現れるまで待つ（仕様書 6.3）。
///
/// `baseline_rowid` は**送信前**の最大 ROWID。これより新しい自分の送信で、
/// 本文が一致するものを探す。
pub fn verify(
    chat_db: &Connection,
    handles: &[String],
    baseline_rowid: i64,
    text: &str,
    timeout: Duration,
) -> Result<Outcome> {
    let started = Instant::now();
    let expected = normalize(text);

    while started.elapsed() < timeout {
        std::thread::sleep(VERIFY_POLL_INTERVAL);

        let new = reader::messages_after(chat_db, handles, baseline_rowid)?;
        if let Some(found) = new
            .iter()
            .filter(|m| m.is_from_me)
            .find(|m| m.body.as_deref().map(normalize).as_deref() == Some(expected.as_str()))
        {
            return Ok(Outcome::Sent { rowid: found.rowid });
        }
    }

    Ok(Outcome::Unverified)
}

/// 比較用に本文をならす。
///
/// 送信時に改行や空白が正規化されることがあるため、そのままでは
/// 一致しない。絵文字や記号は落とさない（別のメッセージと取り違える）。
fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Messages.app が起動しているか。初回の検証タイムアウトを決めるのに使う。
pub fn messages_app_running() -> bool {
    Command::new("pgrep")
        .args(["-x", "Messages"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 検証のタイムアウトを決める。
pub fn verify_timeout() -> Duration {
    if messages_app_running() {
        VERIFY_TIMEOUT
    } else {
        // AppleScript が Messages.app を起動しようとする。その待ち時間を見る。
        VERIFY_TIMEOUT_FIRST
    }
}

/// 埋め込んだスクリプトの位置（デバッグ表示用）。
pub fn script_path_hint() -> &'static Path {
    Path::new("crates/momreply-core/src/imessage/send.applescript")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_recipient_is_rejected() {
        assert!(send("", "やあ").is_err());
        assert!(send("   ", "やあ").is_err());
    }

    /// 空の本文を送ると相手に空のメッセージが届く。防ぐ。
    #[test]
    fn an_empty_body_is_rejected() {
        assert!(send("someone@example.com", "").is_err());
        assert!(send("someone@example.com", "  \n ").is_err());
    }

    #[test]
    fn whitespace_differences_do_not_break_matching() {
        assert_eq!(normalize("ありがとう\nまたね"), normalize("ありがとう またね"));
        assert_eq!(normalize("  はい  "), normalize("はい"));
    }

    /// 絵文字や記号を落とすと、別のメッセージと取り違える。
    #[test]
    fn normalization_keeps_meaningful_characters() {
        assert_ne!(normalize("わかった"), normalize("わかった！"));
        assert_ne!(normalize("うん"), normalize("うん😊"));
    }

    /// 本文は argv で渡す。スクリプトを組み立てないので、
    /// 引用符や改行が入っても壊れない（仕様書 14.6）。
    #[test]
    fn the_script_takes_its_arguments_from_argv() {
        assert!(SEND_SCRIPT.contains("on run argv"));
        assert!(SEND_SCRIPT.contains("item 1 of argv"));
        assert!(SEND_SCRIPT.contains("item 2 of argv"));
        // 本文を埋め込む形跡が無いこと。
        assert!(!SEND_SCRIPT.contains("{text}"));
        assert!(!SEND_SCRIPT.contains("%s"));
    }
}
