use std::sync::atomic::{AtomicU8, Ordering};

static LOCALE: AtomicU8 = AtomicU8::new(0);

/// 0 = ru, 1 = en
pub fn set_locale(code: &str) {
    let value = if code.starts_with("en") {
        1
    } else if code.starts_with("ru") {
        0
    } else {
        // "auto": follow the OS, the frontend sends the resolved language later.
        return;
    };
    LOCALE.store(value, Ordering::Relaxed);
}

pub fn t(key: &str) -> &'static str {
    let en = LOCALE.load(Ordering::Relaxed) == 1;
    match (key, en) {
        ("tray.show", false) => "Показать DwV",
        ("tray.show", true) => "Show DwV",
        ("tray.pause_all", false) => "Пауза всего",
        ("tray.pause_all", true) => "Pause all",
        ("tray.resume_all", false) => "Продолжить всё",
        ("tray.resume_all", true) => "Resume all",
        ("menu.settings", false) => "Настройки",
        ("menu.settings", true) => "Settings",
        ("menu.quit", false) => "Выход",
        ("menu.quit", true) => "Quit",
        _ => "DwV",
    }
}
