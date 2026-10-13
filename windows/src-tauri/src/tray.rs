// Notification-area icon: Open, Weekly recap, Settings, Pause (Resume while
// paused, 1d), Quit Glim.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::i18n::{n_, t};
use crate::island::WINDOW_LABEL;

/// The menu's items, id and English label, in order. The labels are shown in
/// the interface language (i18n.rs) and follow it when it changes.
const ITEMS: [(&str, &str); 5] = [
    ("open", n_("Open Glim")),
    ("recap", n_("Weekly recap")),
    ("settings", n_("Settings…")),
    ("pause", n_("Pause")),
    ("quit", n_("Quit Glim")),
];

/// Kept so a language change relabels the menu without rebuilding the tray.
struct Items(Vec<(&'static str, MenuItem<Wry>)>);

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let mut items = Vec::new();
    for (id, label) in ITEMS {
        items.push((label, MenuItem::with_id(app, id, t(label), true, None::<&str>)?));
    }
    let [open, recap, settings, pause, quit] = [0, 1, 2, 3, 4].map(|i| &items[i].1);
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;

    let menu = Menu::with_items(app, &[open, recap, &sep1, settings, pause, &sep2, quit])?;
    app.manage(Items(items));

    let mut builder = TrayIconBuilder::with_id("glim")
        .tooltip("Glim")
        .menu(&menu)
        .on_menu_event(|app: &AppHandle, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "settings" => crate::show_settings_window(app),
            // Pause is Rust's (pause.rs): the island hears the result.
            "pause" => crate::pause::toggle(app),
            id => {
                // The way back from Hidden (and Ember): opening Glim from the
                // tray brings the pill back first.
                if id == "open" && crate::presence::current(app) != crate::presence::Presence::Pill {
                    crate::presence::set_visibility(app, crate::presence::Visibility::Normal);
                }
                let _ = app.emit_to(WINDOW_LABEL, "tray", id.to_string());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    Ok(())
}

/// The Pause item's label: "Resume" while paused.
fn label_for(label: &'static str, paused: bool) -> &'static str {
    if label == ITEMS[3].1 && paused {
        n_("Resume")
    } else {
        label
    }
}

/// Relabels the menu in the current language.
pub fn retitle(app: &AppHandle) {
    let Some(items) = app.try_state::<Items>() else { return };
    let paused = crate::writing::paused();
    for (label, item) in &items.0 {
        let _ = item.set_text(t(label_for(label, paused)));
    }
}

/// Pause changed: the menu says Resume, and the icon's tooltip "Glim
/// (paused)", which is all that shows in Hidden mode.
pub fn show_paused(app: &AppHandle, paused: bool) {
    retitle(app);
    if let Some(tray) = app.tray_by_id("glim") {
        let tip = if paused { t("Glim (paused)") } else { "Glim".to_string() };
        let _ = tray.set_tooltip(Some(tip));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pause_item_reads_resume_while_paused() {
        assert_eq!(label_for("Pause", true), "Resume");
        assert_eq!(label_for("Pause", false), "Pause");
        assert_eq!(label_for("Open Glim", true), "Open Glim");
    }
}
