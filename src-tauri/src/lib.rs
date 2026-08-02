//! MomReply のメニューバーアプリ。
//!
//! chat.db・app.db・LLM の実装は `momreply-core` にある。
//! 仕様書 4.1 はこれらを `src-tauri/src/` 配下に置いているが、
//! CLI と共有するためクレートを分けている。ここは薄いシェルに留める。

mod commands;
mod notify;
mod tray;
mod watcher;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            tray::setup(app.handle())?;
            // 新着の監視を常駐させる。ここが動いていないと、
            // 材料不足で止まったことに気づけず無返信になる。
            watcher::spawn(app.handle().clone());
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
            commands::list_pending,
            commands::send_reply,
            commands::regenerate,
            commands::skip_pending,
            commands::get_run_mode,
            commands::set_run_mode,
            commands::list_targets,
            commands::list_chat_choices,
            commands::add_target,
            commands::remove_target,
            commands::rebuild_fewshot,
            commands::draft_latest,
            commands::update_target,
            commands::get_limits,
            commands::set_limit,
            commands::list_providers,
            commands::get_primary_provider,
            commands::set_primary_provider,
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
