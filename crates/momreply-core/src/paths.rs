//! アプリのファイル配置（仕様書「名称と識別子」）。
//!
//! bundle identifier はフルディスクアクセスとオートメーション権限の付与単位。
//! **この値を変更すると付与済みの権限がすべて無効になる。**

use std::path::PathBuf;

use anyhow::{Context, Result};

/// bundle identifier。変更しないこと。
pub const BUNDLE_ID: &str = "net.votepurchase.momreply";

fn home() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME が設定されていない")?;
    Ok(PathBuf::from(home))
}

/// `~/Library/Application Support/net.votepurchase.momreply`
pub fn data_dir() -> Result<PathBuf> {
    Ok(home()?.join("Library/Application Support").join(BUNDLE_ID))
}

/// `.../app.db`
pub fn app_db() -> Result<PathBuf> {
    Ok(data_dir()?.join("app.db"))
}

/// `.../config.toml`
pub fn config_file() -> Result<PathBuf> {
    Ok(data_dir()?.join("config.toml"))
}

/// `.../self.md` — 自分側の事実。AI が断定してよい材料。
///
/// 仕様書には無いが、相手の質問に具体的に答えるために必要になる。
/// 相手プロファイル（[`target_profile`]）が「相手について」なのに対し、
/// こちらは「自分について」を持つ。
pub fn self_profile() -> Result<PathBuf> {
    Ok(data_dir()?.join("self.md"))
}

/// `.../targets/<slug>.md` — 相手ごとのプロファイル。
pub fn target_profile(slug: &str) -> Result<PathBuf> {
    Ok(data_dir()?.join("targets").join(format!("{slug}.md")))
}

/// 上記ディレクトリを作る。
pub fn ensure_dirs() -> Result<()> {
    let dir = data_dir()?;
    std::fs::create_dir_all(dir.join("targets"))
        .with_context(|| format!("データディレクトリを作れない: {}", dir.display()))?;
    Ok(())
}
