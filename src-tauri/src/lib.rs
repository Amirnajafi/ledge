mod audio;
mod bluetooth;
mod camera;
mod clip;
mod diag;
mod display;
mod dock;
mod draw;
mod files;
mod media;
mod meet;
mod mouse;
#[cfg(target_os = "macos")]
mod multitouch;
mod permissions;
mod sync;
mod trackpad;
mod tray;
mod update;

use tauri::Manager;

#[tauri::command]
fn quit(app: tauri::AppHandle) {
    app.exit(0);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if std::env::var_os(update::AFTER_UPDATE).is_some() {
        std::thread::sleep(std::time::Duration::from_secs(2)); // the old copy is still quitting
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            diag::init(app.handle());
            update::start(app.handle());
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let window = app.get_webview_window("main").ok_or("main window missing")?;
            dock::install_glass(&window);
            tray::install(app.handle())?;
            mouse::start();
            trackpad::start(app.handle());
            let sync = sync::start(app.handle());
            app.manage(sync);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            quit,
            diag::log_line,
            dock::dock_place,
            dock::dock_dropped,
            audio::audio_state,
            audio::audio_set_default,
            audio::audio_set_mute,
            media::media_key,
            meet::meet_camera,
            draw::draw_set,
            draw::draw_style,
            mouse::mouse_configure,
            mouse::mouse_status,
            bluetooth::batteries,
            trackpad::trackpad_configure,
            sync::sync_status,
            sync::sync_set_enabled,
            sync::files_send,
            sync::settings_push,
            sync::update_from,
            update::update_check,
            files::files_reveal,
            permissions::permissions,
            permissions::permission_open,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
