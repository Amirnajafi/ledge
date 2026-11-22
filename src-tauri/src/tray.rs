//! Menu bar icon. Add items to `menu()` and handle their ids in `on_menu`.

use tauri::image::Image;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Runtime};

fn menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let version = MenuItem::with_id(app, "version", format!("Ledge {}", crate::update::VERSION), false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Ledge", true, Some("CmdOrCtrl+Q"))?;
    Menu::with_items(app, &[&version, &PredefinedMenuItem::separator(app)?, &quit])
}

fn on_menu<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    if event.id() == "quit" {
        app.exit(0);
    }
}

pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    TrayIconBuilder::with_id("ledge")
        .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
        .icon_as_template(true) // macOS tints it for light/dark menu bars
        .tooltip("Ledge")
        .menu(&menu(app)?)
        .show_menu_on_left_click(true)
        .on_menu_event(on_menu)
        .build(app)?;
    Ok(())
}
