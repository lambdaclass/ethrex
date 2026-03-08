mod ai_provider;
mod appchain_manager;
mod commands;
mod deployment_db;
mod local_server;
mod pilot_memory;
mod process_manager;
mod runner;
mod telegram_bot;

use commands::*;
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager, WindowEvent,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(process_manager::ProcessManager::new())
        .manage(Arc::new(appchain_manager::AppchainManager::new()))
        .manage(Arc::new(runner::ProcessRunner::new()))
        .manage(Arc::new(ai_provider::AiProvider::new()))
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // Auto-start local server for deployment management
            let server = Arc::new(local_server::LocalServer::new());
            app.manage(server.clone());
            tauri::async_runtime::spawn(async move {
                if let Err(e) = server.start().await {
                    log::warn!("Failed to auto-start local server: {e}");
                }
            });

            // System tray
            let show_item =
                MenuItem::with_id(app, "show", "Tokamak Appchain 열기", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Tokamak Appchain")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|_tray, _event| {
                    // Only open via menu "열기", not on tray icon click
                })
                .build(app)?;

            // Pilot memory (persistent chat/event storage)
            let memory = Arc::new(pilot_memory::PilotMemory::new());

            // Telegram bot manager (dynamic start/stop from Settings)
            let ai = app.state::<Arc<ai_provider::AiProvider>>().inner().clone();
            let am = app.state::<Arc<appchain_manager::AppchainManager>>().inner().clone();
            let runner = app.state::<Arc<runner::ProcessRunner>>().inner().clone();
            let tg_manager = Arc::new(telegram_bot::TelegramBotManager::new(
                ai, am.clone(), runner.clone(), memory.clone(),
            ));
            // Auto-start if configured
            if let Ok(()) = tg_manager.start() {
                log::info!("Telegram bot auto-started");
            }
            // Start background health monitor
            let tg_monitor = tg_manager.clone();
            tauri::async_runtime::spawn(
                telegram_bot::TelegramBotManager::health_monitor(
                    am, runner, memory, tg_monitor,
                )
            );
            app.manage(tg_manager);

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Hide instead of close, keep tray alive
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_ai_config,
            save_ai_config,
            has_ai_key,
            fetch_ai_models,
            test_ai_connection,
            disconnect_ai,
            get_ai_mode,
            set_ai_mode,
            get_token_usage,
            get_all_status,
            start_node,
            stop_node,
            get_node_status,
            get_logs,
            send_chat_message,
            create_appchain,
            list_appchains,
            get_appchain,
            delete_appchain,
            start_appchain_setup,
            get_setup_progress,
            stop_appchain,
            update_appchain_public,
            get_chat_context,
            start_local_server,
            stop_local_server,
            get_local_server_status,
            open_deployment_ui,
            save_platform_token,
            get_platform_token,
            delete_platform_token,
            start_platform_login,
            poll_platform_login,
            get_platform_user,
            list_docker_deployments,
            delete_docker_deployment,
            stop_docker_deployment,
            start_docker_deployment,
            get_docker_containers,
            get_telegram_config,
            save_telegram_config,
            toggle_telegram_bot,
            get_telegram_bot_status,
            send_telegram_notification,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
