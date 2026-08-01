//! chat.db への read-only 接続。
//!
//! このモジュール以外から chat.db への `Connection` を作らないこと。
//! 書き込み可能なフラグでの接続は仕様上の重大バグである（仕様書 5.1）。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OpenFlags};

/// `~/Library/Messages/chat.db` の既定パス。
pub fn default_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME が設定されていない")?;
    Ok(Path::new(&home).join("Library/Messages/chat.db"))
}

/// chat.db を read-only で開く。
///
/// `SQLITE_OPEN_READ_ONLY` のみを渡す。CREATE も WRITE も付けない。
/// 開いた直後に SQLite 側にも read-only であることを問い合わせて二重に確認する。
pub fn open_readonly(path: &Path) -> Result<Connection> {
    if !path.is_file() {
        bail!("chat.db が見つからない: {}", path.display());
    }

    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .with_context(|| {
        format!(
            "chat.db を開けない: {}\n\
             フルディスクアクセスが付与されているか確認すること。\n\
             開発中は cargo を起動しているターミナル / IDE 側に付与し、\n\
             付与後にそのアプリを再起動する必要がある（仕様書 14.4 / 14.5）。",
            path.display()
        )
    })?;

    // 念のため SQLite 自身に確認する。ここが false なら即座に落とす。
    if !conn.is_readonly("main")? {
        bail!("chat.db が read-only で開かれていない。処理を中止する");
    }

    Ok(conn)
}
