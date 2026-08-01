//! メニューバーのアイコンとポップオーバー（仕様書 6.6）。

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WebviewWindow,
};
use tauri_plugin_positioner::{Position, WindowExt};

const POPOVER: &str = "popover";

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let settings = MenuItem::with_id(app, "settings", "設定を開く", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "MomReply を終了", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&settings, &separator, &quit])?;

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().expect("アイコンが無い"))
        // メニューバーのアイコンはテンプレート画像にする。
        // ライト / ダークの切り替えに OS 側が追従してくれる。
        .icon_as_template(true)
        // 左クリックはポップオーバーの開閉に使うのでメニューを出さない。
        // メニューは右クリックで出す。
        .show_menu_on_left_click(false)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "settings" => {
                if let Some(window) = app.get_webview_window(POPOVER) {
                    show_popover(&window);
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);

            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let Some(window) = tray.app_handle().get_webview_window(POPOVER) else {
                    return;
                };
                // トグルにする。でないとクリックのたびに前面へ出続けて
                // 閉じられなくなる。
                if window.is_visible().unwrap_or(false) {
                    let _ = window.hide();
                } else {
                    show_popover(&window);
                }
            }
        })
        .build(app)?;

    Ok(())
}

fn show_popover(window: &WebviewWindow) {
    // トレイアイコンの直下に出す。
    let _ = window.move_window(Position::TrayBottomCenter);
    let _ = window.show();
    let _ = window.set_focus();
}
