//! The macOS menu bar. Its own items send their id to the UI as a
//! `satchel://menu` event, which runs the same action as the button or
//! shortcut (`ui/src/app/menu.ts`); the system items (Edit, Window, Hide,
//! Quit) are handled natively.
//!
//! Key equivalents reach the web page first, so where the UI already binds a
//! ⌘ shortcut (⌘K, ⌘\) the page handles the key and the menu only fires on a
//! click. The rest (⌘R, ⇧⌘U, …) are menu-only, like Apple Mail's.

use tauri::menu::{AboutMetadata, HELP_SUBMENU_ID, Menu, MenuItemBuilder, SubmenuBuilder, WINDOW_SUBMENU_ID};
use tauri::{AppHandle, Emitter, Runtime};

/// Builds the menu bar. Ids starting `key:` run the UI's binding for that key
/// (see `bind` in `ui/src/lib/ui.ts`); the others are named actions.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let item = |id: &str, label: &str, accel: Option<&str>| {
        let b = MenuItemBuilder::with_id(id, label);
        match accel {
            Some(a) => b.accelerator(a),
            None => b,
        }
        .build(app)
    };
    let info = app.package_info();
    let about = AboutMetadata {
        name: Some(info.name.clone()),
        version: Some(info.version.to_string()),
        copyright: app.config().bundle.copyright.clone(),
        ..Default::default()
    };

    let app_menu = SubmenuBuilder::new(app, &info.name)
        .about(Some(about))
        .separator()
        .item(&item("key:g ,", "Settings…", Some("Cmd+,"))?)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    let file = SubmenuBuilder::new(app, "File")
        .item(&item("compose", "New Message", Some("Cmd+N"))?)
        .item(&item("new-event", "New Event", None)?)
        .item(&item("new-contact", "New Contact", None)?)
        .separator()
        .item(&item("sync", "Sync All Accounts", Some("Shift+Cmd+N"))?)
        .build()?;

    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .separator()
        .item(&item("key:/", "Search Mail", Some("Alt+Cmd+F"))?)
        .build()?;

    let view = SubmenuBuilder::new(app, "View")
        .item(&item("go:mail", "Mail", Some("Cmd+1"))?)
        .item(&item("go:calendar", "Calendar", Some("Cmd+2"))?)
        .item(&item("go:contacts", "Contacts", Some("Cmd+3"))?)
        .item(&item("go:review", "AI Proposals", Some("Cmd+4"))?)
        .separator()
        .item(&item("key:mod+k", "Command Palette…", Some("Cmd+K"))?)
        .item(&item("key:mod+\\", "Show or Hide Sidebar", Some("Cmd+\\"))?)
        .item(&item("theme", "Toggle Dark Mode", None)?)
        .build()?;

    let mailbox = SubmenuBuilder::new(app, "Mailbox")
        .item(&item("key:g i", "Go to Inbox", None)?)
        .item(&item("key:g s", "Go to Flagged", None)?)
        .item(&item("key:g t", "Go to Sent", None)?)
        .item(&item("key:g d", "Go to Drafts", None)?)
        .build()?;

    let message = SubmenuBuilder::new(app, "Message")
        .item(&item("key:r", "Reply", Some("Cmd+R"))?)
        .item(&item("key:a", "Reply All", Some("Shift+Cmd+R"))?)
        .item(&item("key:f", "Forward", Some("Shift+Cmd+F"))?)
        .separator()
        .item(&item("key:e", "Archive", Some("Ctrl+Cmd+A"))?)
        .item(&item("key:v", "Move to…", None)?)
        .item(&item("key:#", "Delete", Some("Cmd+Backspace"))?)
        .item(&item("key:!", "Mark as Spam / Not Spam", Some("Shift+Cmd+J"))?)
        .separator()
        .item(&item("key:u", "Mark as Unread", Some("Shift+Cmd+U"))?)
        .item(&item("key:shift+i", "Mark as Read", None)?)
        .item(&item("key:s", "Flag / Unflag", Some("Shift+Cmd+L"))?)
        .build()?;

    let window = SubmenuBuilder::with_id(app, WINDOW_SUBMENU_ID, "Window")
        .minimize()
        .maximize()
        .build()?;

    let help = SubmenuBuilder::with_id(app, HELP_SUBMENU_ID, "Help")
        .item(&item("key:?", "Keyboard Shortcuts", Some("Cmd+/"))?)
        .build()?;

    Menu::with_items(app, &[&app_menu, &file, &edit, &view, &mailbox, &message, &window, &help])
}

/// Hands a click on one of our items to the UI.
pub fn forward<R: Runtime>(app: &AppHandle<R>, id: &str) {
    let _ = app.emit_to("main", "satchel://menu", id);
}

