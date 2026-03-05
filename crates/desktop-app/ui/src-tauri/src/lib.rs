mod commands;
mod process_manager;

use commands::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .manage(process_manager::ProcessManager::new())
        .invoke_handler(tauri::generate_handler![
            get_all_status,
            start_node,
            stop_node,
            get_node_status,
            get_logs,
            send_chat_message,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
