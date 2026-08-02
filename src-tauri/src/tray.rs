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
        // アプリアイコンは流用できない。テンプレート画像はアルファだけを
        // 見るため、色で描いた「抜き」（吹き出しの中の点）が潰れてしまう。
        // トレイ用は抜きたい部分が実際に透明な専用画像を使う。
        //
        // 処理中はここが差し替わる（[`crate::activity`]）。
        .icon(tauri::image::Image::from_bytes(crate::activity::ICON_IDLE)?)
        // ライト / ダークの切り替えは OS 側に任せる。
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
