//! MomReply のメニューバーアプリ。
//!
//! chat.db・app.db・LLM の実装は `momreply-core` にある。
//! 仕様書 4.1 はこれらを `src-tauri/src/` 配下に置いているが、
//! CLI と共有するためクレートを分けている。ここは薄いシェルに留める。

mod commands;
mod tray;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_positioner::init())
        .setup(|app| {
            tray::setup(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::set_api_key,
            commands::get_key_status,
            commands::list_key_statuses,
            commands::delete_api_key,
            commands::verify_api_key,
            commands::can_enable_auto_send,
            commands::list_models,
            commands::set_model,
            commands::get_self_profile,
            commands::set_self_profile,
            commands::self_profile_path,
            commands::list_fact_candidates,
            commands::approve_fact,
            commands::reject_fact,
        ])
        .build(tauri::generate_context!())
        .expect("Tauri アプリを初期化できない")
        .run(|_app, event| {
            // ウィンドウを閉じてもアプリは常駐させる。
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                api.prevent_exit();
            }
        });
}
