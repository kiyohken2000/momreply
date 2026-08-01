//! chat.db（読み取り専用）へのアクセス。
//!
//! chat.db への接続は [`connection::open_readonly`] だけが作る。
//! 他の場所で `Connection::open` 系を呼ばないこと（仕様書 5.1）。

pub mod connection;
pub mod reader;

pub use connection::{default_path, open_readonly};
pub use reader::{list_chats, max_rowid, messages_after, recent_messages, ChatSummary, Message, SkipReason};
