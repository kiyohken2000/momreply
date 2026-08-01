//! API キーの保管（仕様書 7.5）。
//!
//! # 設計の要点
//!
//! **このモジュールはキー本体を外に出さない。** 公開しているのは
//! [`KeyStatus`] と「キーを使って何かする」関数だけで、キーを返す
//! 関数は無い。仕様書 7.5.3 が「キー本体を戻り値に含むコマンドを
//! 作らないこと。デバッグ目的でも作らない」としているため、
//! UI 層が誤って露出させる余地を型で塞いでいる。
//!
//! キーを読めるのは同じモジュール内の [`with_key`] 経由のみで、
//! これは `pub(crate)` である。

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::Provider;

/// Keychain の service 名。bundle identifier と同じ（仕様書「名称と識別子」）。
const SERVICE: &str = crate::paths::BUNDLE_ID;

/// UI に渡してよい状態。**キー本体は含まない。**
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyStatus {
    pub provider: String,
    /// Keychain に保存されているか。
    pub configured: bool,
    /// 疎通テストに成功しているか。自動送信の前提条件に使う。
    pub verified: bool,
    /// 末尾 4 文字のみ（`sk-...a3f9`）。
    ///
    /// **先頭は絶対に含めない。** 前方一致で漏れると意味がない
    /// （仕様書 7.5.2）。
    pub masked: Option<String>,
    pub last_verified_at: Option<i64>,
    /// 検証に失敗した理由。UI に出す。
    pub error: Option<String>,
}

impl KeyStatus {
    pub fn unset(provider: Provider) -> Self {
        KeyStatus {
            provider: provider.id().to_string(),
            configured: false,
            verified: false,
            masked: None,
            last_verified_at: None,
            error: None,
        }
    }
}

/// 末尾 4 文字だけを見せる形に落とす。
///
/// 4 文字に満たないキーは伏せ字のみを返す。短いキーで全体が
/// 見えてしまうのを防ぐ。
pub fn mask(key: &str) -> String {
    let count = key.chars().count();
    if count <= 4 {
        return "•".repeat(count.max(1));
    }
    let tail: String = key.chars().skip(count - 4).collect();
    format!("•••{tail}")
}

fn entry(provider: Provider) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, provider.keychain_account())
        .with_context(|| format!("Keychain を開けない（service={SERVICE}）"))
}

/// キーを保存する。前後の空白と改行は落とす（仕様書 7.5.4）。
pub fn set(provider: Provider, key: &str) -> Result<()> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        bail!("APIキーが空です");
    }
    entry(provider)?
        .set_password(trimmed)
        .with_context(|| format!("{} のキーを Keychain に保存できない", provider.id()))?;
    Ok(())
}

/// キーが保存されているかと、そのマスク表示。
pub fn status(provider: Provider) -> KeyStatus {
    match read(provider) {
        Ok(Some(key)) => KeyStatus {
            provider: provider.id().to_string(),
            configured: true,
            verified: false,
            masked: Some(mask(&key)),
            last_verified_at: None,
            error: None,
        },
        Ok(None) => KeyStatus::unset(provider),
        Err(why) => KeyStatus {
            error: Some(why.to_string()),
            ..KeyStatus::unset(provider)
        },
    }
}

pub fn delete(provider: Provider) -> Result<()> {
    match entry(provider)?.delete_credential() {
        Ok(()) => Ok(()),
        // 元から無いなら成功とみなす。
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(why) => Err(why).context("Keychain から削除できない"),
    }
}

pub fn is_configured(provider: Provider) -> bool {
    matches!(read(provider), Ok(Some(_)))
}

/// キーを使って処理を行う。**キーはこのクロージャの外に出さないこと。**
///
/// `pub(crate)` にしてあるのは、UI 層からキーを取り出す経路を作らせない
/// ため（仕様書 7.5.2 / 7.5.3）。
pub(crate) fn with_key<T>(provider: Provider, f: impl FnOnce(&str) -> T) -> Result<T> {
    let key = read(provider)?
        .with_context(|| format!("{} の API キーが設定されていません", provider.id()))?;
    Ok(f(&key))
}

fn read(provider: Provider) -> Result<Option<String>> {
    // 開発中は Keychain のアイテムがコード署名に紐づき、dev ビルドの
    // 署名が毎回変わるため読み取りに失敗する（仕様書 7.5.7）。
    // リリースビルドには絶対に含めない。
    #[cfg(debug_assertions)]
    if let Some(key) = dev_override(provider) {
        return Ok(Some(key));
    }

    match entry(provider)?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(why) => Err(why).context("Keychain から読み取れない"),
    }
}

#[cfg(debug_assertions)]
fn dev_override(provider: Provider) -> Option<String> {
    let var = format!("MOMREPLY_DEV_API_KEY_{}", provider.id().to_uppercase());
    std::env::var(var).ok().filter(|v| !v.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 先頭が漏れると前方一致で絞り込めてしまう（仕様書 7.5.2）。
    #[test]
    fn mask_never_reveals_the_beginning() {
        let masked = mask("sk-ant-api03-SECRETSECRET-a3f9");
        assert!(masked.ends_with("a3f9"));
        assert!(!masked.contains("sk-"));
        assert!(!masked.contains("SECRET"));
    }

    #[test]
    fn mask_hides_short_keys_entirely() {
        assert_eq!(mask("ab"), "••");
        assert_eq!(mask("abcd"), "••••");
        assert!(!mask("abcd").contains('a'));
    }

    #[test]
    fn unset_status_carries_no_secret() {
        let s = KeyStatus::unset(Provider::Anthropic);
        assert!(!s.configured);
        assert!(!s.verified);
        assert_eq!(s.masked, None);
    }
}
