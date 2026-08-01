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

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::Provider;

/// プロセス内のキャッシュ。
///
/// # なぜ持つか
///
/// Keychain を読むたびに、macOS はアクセス許可のダイアログを出しうる。
/// dev ビルドは再リンクのたびに ad-hoc 署名のハッシュが変わるため、
/// 「常に許可」を押しても次のビルドで無効になる（仕様書 7.5.7）。
/// キャッシュが無いと、リトライやプロバイダ一覧の表示のたびに
/// ダイアログが出て操作にならない。
///
/// # 安全性について
///
/// キーはどのみち HTTP リクエストを組む間メモリ上に載る。
/// ここで保持しても露出面は増えない。キャッシュはこのモジュールの
/// 内側にあり、[`with_key`] 以外から取り出す経路は無い。
fn cache() -> &'static Mutex<HashMap<&'static str, Option<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, Option<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached(provider: Provider) -> Option<Option<String>> {
    cache().lock().ok()?.get(provider.id()).cloned()
}

fn remember(provider: Provider, value: Option<String>) {
    if let Ok(mut c) = cache().lock() {
        c.insert(provider.id(), value);
    }
}

fn forget(provider: Provider) {
    if let Ok(mut c) = cache().lock() {
        c.remove(provider.id());
    }
}

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
    // 保存した値をそのまま覚えておく。直後の疎通テストで読み直すと
    // また許可を求められるため。
    remember(provider, Some(trimmed.to_string()));
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
    forget(provider);
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

    // Keychain に触る回数を 1 プロセス 1 回に抑える。
    // 読むたびに許可ダイアログが出る環境では、これが無いと操作にならない。
    if let Some(hit) = cached(provider) {
        return Ok(hit);
    }

    let value = match entry(provider)?.get_password() {
        Ok(key) => Some(key),
        Err(keyring::Error::NoEntry) => None,
        Err(why) => return Err(why).context("Keychain から読み取れない"),
    };
    remember(provider, value.clone());
    Ok(value)
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
