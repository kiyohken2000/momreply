//! MomReply のコア。chat.db 読み取り・app.db・返信生成を担う。
//!
//! UI（Tauri / CLI）はこのクレートを呼ぶだけにして、
//! chat.db へのアクセス経路を [`imessage`] に一本化する。

pub mod imessage;
pub mod paths;
pub mod store;
