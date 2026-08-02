//! いま裏で何をしているか。
//!
//! 自動送信は 1 往復に 1 分半ほどかかる。連投がそろうのを 45 秒待ち、
//! 生成に 50 秒かかるためである。その間ポップオーバーを開いても、
//! これまでは「確認待ちの返信はありません」としか出なかった。
//! 止まっているのか動いているのか区別がつかない。
//!
//! 状態は 1 つだけ持ち、**書くのは監視スレッドだけ**にしてある。
//! 手で押したときの生成（`draft_latest` など）はボタン自身が状態を出すし、
//! ここに書きに来ると、監視が始めた処理を横から消してしまう。

use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// 状態が変わったことを知らせる合図。
pub const EVENT_ACTIVITY: &str = "momreply://activity";

/// 何をしているか。**フロントに出す文言はここで決める。**
/// 画面側に分岐を持たせると、増やすたびに 2 か所直すことになる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// 連投がそろうのを待っている。
    Settling,
    /// 返信案を作っている。
    Generating,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Phase::Settling => "続きが来ないか待っています",
            Phase::Generating => "返信を作っています",
        }
    }
}

/// メニューバーのアイコン。**テンプレート画像はアルファしか見ない**ので、
/// 色ではなく輪郭そのものを変えて区別する。22px まで縮んでも分かるよう、
/// 中身の詰まり方で差をつけてある。
///
/// - 待受中: 吹き出しに点が 3 つ（何も起きていない）
/// - 待機中: 輪郭だけ（中身がまだ無い）
/// - 生成中: 塗りつぶし（中身が詰まっている）
pub const ICON_IDLE: &[u8] = include_bytes!("../icons/tray.png");
const ICON_SETTLING: &[u8] = include_bytes!("../icons/tray-settling.png");
const ICON_WORKING: &[u8] = include_bytes!("../icons/tray-working.png");

fn icon_for(activity: Option<&Activity>) -> &'static [u8] {
    match activity.map(|a| a.phase) {
        Some(Phase::Settling) => ICON_SETTLING,
        Some(Phase::Generating) => ICON_WORKING,
        None => ICON_IDLE,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Activity {
    pub who: String,
    pub phase: Phase,
    pub label: &'static str,
}

fn cell() -> &'static Mutex<Option<Activity>> {
    static CURRENT: OnceLock<Mutex<Option<Activity>>> = OnceLock::new();
    CURRENT.get_or_init(|| Mutex::new(None))
}

/// 状態を差し替えて合図を出す。**同じ状態なら何もしない。**
/// 監視は数秒おきに回るので、毎周期で合図を出すと画面が点滅する。
pub fn set(app: &AppHandle, who: &str, phase: Phase) {
    let next = Activity {
        who: who.to_string(),
        phase,
        label: phase.label(),
    };
    {
        let mut slot = cell().lock().unwrap_or_else(|e| e.into_inner());
        if slot
            .as_ref()
            .is_some_and(|a| a.phase == next.phase && a.who == next.who)
        {
            return;
        }
        *slot = Some(next.clone());
    }
    announce(app, Some(&next));
}

pub fn clear(app: &AppHandle) {
    {
        let mut slot = cell().lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            return;
        }
        *slot = None;
    }
    announce(app, None);
}

pub fn current() -> Option<Activity> {
    cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .cloned()
}

/// フロントとメニューバーの両方へ反映する。
///
/// ポップオーバーを開いていないときは、メニューバーだけが手がかりになる。
/// アイコンの形で今の状態を示し、詳しくはツールチップに出す。
fn announce(app: &AppHandle, activity: Option<&Activity>) {
    let _ = app.emit(EVENT_ACTIVITY, activity);

    let Some(tray) = app.tray_by_id("main") else {
        return;
    };

    let tooltip = match activity {
        Some(a) => format!("MomReply — {}（{}）", a.label, a.who),
        None => "MomReply".to_string(),
    };
    let _ = tray.set_tooltip(Some(&tooltip));

    if let Ok(image) = tauri::image::Image::from_bytes(icon_for(activity)) {
        let _ = tray.set_icon(Some(image));
        // set_icon はテンプレート指定を引き継がない。指定し直さないと
        // ダークモードで色が反転しなくなる。
        let _ = tray.set_icon_as_template(true);
    }
}
