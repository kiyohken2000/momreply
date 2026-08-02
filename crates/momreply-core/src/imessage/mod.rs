//! chat.db（読み取り専用）へのアクセス。
//!
//! chat.db への接続は [`connection::open_readonly`] だけが作る。
//! 他の場所で `Connection::open` 系を呼ばないこと（仕様書 5.1）。

pub mod connection;
pub mod reader;
pub mod sender;
pub mod watcher;

pub use connection::{default_path, open_readonly};
pub use reader::{
    count_own_replies_after, list_chats, max_rowid, messages_after, recent_messages, ChatSummary,
    Message, SkipReason,
};
pub use watcher::{
    burst, burst_text, gap_detected, is_stale, plan, plan_with_burst, Passed, Plan, BURST_WINDOW,
};
