// Notification-area icon: Open, Weekly recap, Pick up where you left off
// (the resume card, decided 2026-10-15), Settings, Pause (Resume while
// paused, 1d), Allow screenshots for 10 minutes (Hide from screenshots again
// while allowed; screen_capture.rs, 2026-11-14), Stop remote tasks (the
// remote kill switch, 2026-11-14), Quit Glim.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::i18n::{n_, t};
use crate::island::WINDOW_LABEL;

/// The menu's items, id and English label, in order. The labels are shown in
/// the interface language (i18n.rs) and follow it when it changes.
const ITEMS: [(&str, &str); 8] = [
    ("open", n_("Open Glim")),
    ("recap", n_("Weekly recap")),
    ("resume", n_("Pick up where you left off")),
    ("settings", n_("Settings…")),
    ("pause", PAUSE),
    ("capture", CAPTURE),
    // The remote courier's kill switch (remote/): one click refuses every
    // remote task until it is switched back in Settings → Remote.
    ("remote-kill", n_("Stop remote tasks")),
    ("quit", n_("Quit Glim")),
];

/// Kept so a language change relabels the menu without rebuilding the tray.
struct Items(Vec<(&'static str, MenuItem<Wry>)>);

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let mut items = Vec::new();
    for (id, label) in ITEMS {
        items.push((label, MenuItem::with_id(app, id, t(label), true, None::<&str>)?));
    }
    let [open, recap, resume, settings, pause, capture, remote_kill, quit] = [0, 1, 2, 3, 4, 5, 6, 7].map(|i| &items[i].1);
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;

    let menu = Menu::with_items(app, &[open, recap, resume, &sep1, settings, pause, capture, remote_kill, &sep2, quit])?;
    app.manage(Items(items));

    let mut builder = TrayIconBuilder::with_id("glim")
        .tooltip("Glim")
        .menu(&menu)
        .on_menu_event(|app: &AppHandle, event| match event.id.as_ref() {
            "quit" => {
                crate::ai::budget::on_quit();
                #[cfg(windows)]
                crate::voice::whisper::stop();
                app.exit(0);
            }
            "settings" => crate::show_settings_window(app, "tray"),
            // Pause is Rust's (pause.rs): the island hears the result.
            "pause" => crate::pause::toggle(app),
            "remote-kill" => crate::remote_kill_from_tray(app),
            "capture" => {
                if crate::screen_capture::allowed_until() > 0 {
                    crate::screen_capture::end(app, "tray");
                } else {
                    crate::screen_capture::allow_for_a_while(app);
                }
            }
            id => {
                // The way back from Hidden (and Ember): opening Glim, or the
                // resume card, from the tray brings the pill back first.
                if (id == "open" || id == "resume") && crate::presence::current(app) != crate::presence::Presence::Pill {
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

const PAUSE: &str = n_("Pause");
const CAPTURE: &str = n_("Allow screenshots for 10 minutes");

/// An item's label now: "Resume" while paused, "Hide from screenshots
/// again" while screenshots are allowed.
fn label_for(label: &'static str, paused: bool, capture_allowed: bool) -> &'static str {
    if label == PAUSE && paused {
        n_("Resume")
    } else if label == CAPTURE && capture_allowed {
        n_("Hide from screenshots again")
    } else {
        label
    }
}

/// Relabels the menu in the current language.
pub fn retitle(app: &AppHandle) {
    let Some(items) = app.try_state::<Items>() else { return };
    let paused = crate::writing::paused();
    let allowed = crate::screen_capture::allowed_until() > 0;
    for (label, item) in &items.0 {
        let _ = item.set_text(t(label_for(label, paused, allowed)));
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
        assert_eq!(label_for("Pause", true, false), "Resume");
        assert_eq!(label_for("Pause", false, false), "Pause");
        assert_eq!(label_for("Open Glim", true, true), "Open Glim");
    }

    #[test]
    fn the_screenshots_item_reads_hide_again_while_allowed() {
        assert_eq!(label_for(CAPTURE, false, false), "Allow screenshots for 10 minutes");
        assert_eq!(label_for(CAPTURE, false, true), "Hide from screenshots again");
        assert_eq!(label_for(PAUSE, false, true), "Pause");
    }
}
