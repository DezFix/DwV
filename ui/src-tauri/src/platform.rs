//! Windows integration: hidden consoles, protocol handlers, autostart and
//! "show in folder". Other platforms get no-ops so the crate still builds and
//! tests there.

#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
pub fn hide_console(command: &mut tokio::process::Command) {
    use std::os::windows::process::CommandExt;
    command.as_std_mut().creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub fn hide_console(_command: &mut tokio::process::Command) {}

#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// Adds or removes the current exe from the Windows Run key. Reads the key
/// first, so the usual "nothing changed" case spawns one `reg query` only.
pub fn apply_autostart(enabled: bool) {
    #[cfg(windows)]
    {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let command = format!(r#""{}" --autostart"#, exe.to_string_lossy());
        let current = reg_query(RUN_KEY, Some("DwV"));
        if enabled {
            if current.as_deref() != Some(command.as_str()) {
                reg_add(RUN_KEY, Some("DwV"), &command);
            }
        } else if current.is_some() {
            reg_run(&["delete", RUN_KEY, "/v", "DwV", "/f"]);
        }
    }
    #[cfg(not(windows))]
    let _ = enabled;
}

/// Runs a registry update on a background thread: `reg.exe` calls must not
/// delay the window or the event loop.
pub fn spawn_registry_setup(autostart: Option<bool>, protocols: bool) {
    std::thread::spawn(move || {
        if protocols {
            register_protocol();
        }
        if let Some(enabled) = autostart {
            apply_autostart(enabled);
        }
    });
}

/// Registers `dwv://` and `magnet:` for the current executable (Windows).
///
/// The magnet handler needs a ProgID listed in `RegisteredApplications`:
/// Windows' "Open with" dialog lists handlers from app capabilities, not from
/// the merged `magnet` class alone. The plain `magnet` class is only claimed
/// when it is free or already ours, so another torrent client the user chose
/// keeps its association.
pub fn register_protocol() {
    #[cfg(windows)]
    {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let exe = exe.to_string_lossy().to_string();
        // Fast path: already registered for this exe.
        if reg_query(r"HKCU\Software\DwV", Some("RegisteredExe")).as_deref() == Some(exe.as_str()) {
            return;
        }

        let icon = format!("\"{exe}\"");
        let command = format!("\"{exe}\" \"%1\"");

        // dwv:// is ours alone, a direct class is enough.
        reg_add(r"HKCU\Software\Classes\dwv", None, "URL:dwv Protocol");
        reg_add(r"HKCU\Software\Classes\dwv", Some("URL Protocol"), "");
        reg_add(r"HKCU\Software\Classes\dwv\DefaultIcon", None, &icon);
        reg_add(
            r"HKCU\Software\Classes\dwv\shell\open\command",
            None,
            &command,
        );

        // magnet: a proper ProgID plus an OpenWith entry.
        reg_add(
            r"HKCU\Software\Classes\DwV.magnet",
            None,
            "URL:magnet Protocol",
        );
        reg_add(
            r"HKCU\Software\Classes\DwV.magnet",
            Some("URL Protocol"),
            "",
        );
        reg_add(r"HKCU\Software\Classes\DwV.magnet\DefaultIcon", None, &icon);
        reg_add(
            r"HKCU\Software\Classes\DwV.magnet\shell\open\command",
            None,
            &command,
        );
        reg_add(
            r"HKCU\Software\Classes\magnet\OpenWithProgids",
            Some("DwV.magnet"),
            "",
        );

        // The direct class lets a fresh install open magnets before the user
        // picks a default application. Never take it from another client.
        let magnet_command = r"HKCU\Software\Classes\magnet\shell\open\command";
        // HKCR is the merged view: a client registered machine-wide counts too.
        let current = reg_query(r"HKCR\magnet\shell\open\command", None).unwrap_or_default();
        if current.trim().is_empty() || current.to_ascii_lowercase().contains("dwv") {
            reg_add(r"HKCU\Software\Classes\magnet", None, "URL:magnet Protocol");
            reg_add(r"HKCU\Software\Classes\magnet", Some("URL Protocol"), "");
            reg_add(r"HKCU\Software\Classes\magnet\DefaultIcon", None, &icon);
            reg_add(magnet_command, None, &command);
        }

        // Default Apps entry (Windows lists these in the "Open with" dialog).
        reg_add(
            r"HKCU\Software\DwV\Capabilities",
            Some("ApplicationName"),
            "DwV",
        );
        reg_add(
            r"HKCU\Software\DwV\Capabilities",
            Some("ApplicationDescription"),
            "DwV Download Manager",
        );
        reg_add(
            r"HKCU\Software\DwV\Capabilities",
            Some("ApplicationIcon"),
            &icon,
        );
        reg_add(
            r"HKCU\Software\DwV\Capabilities\UrlAssociations",
            Some("magnet"),
            "DwV.magnet",
        );
        reg_add(
            r"HKCU\Software\DwV\Capabilities\UrlAssociations",
            Some("dwv"),
            "dwv",
        );
        reg_add(
            r"HKCU\Software\RegisteredApplications",
            Some("DwV"),
            r"Software\DwV\Capabilities",
        );
        reg_add(r"HKCU\Software\DwV", Some("RegisteredExe"), &exe);
    }
}

#[cfg(windows)]
fn reg_run(args: &[&str]) -> Option<std::process::Output> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("reg")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
}

#[cfg(windows)]
fn reg_add(key: &str, name: Option<&str>, value: &str) {
    let mut args = vec!["add", key];
    match name {
        Some(name) => args.extend(["/v", name]),
        None => args.push("/ve"),
    }
    args.extend(["/t", "REG_SZ", "/d", value, "/f"]);
    let _ = reg_run(&args);
}

/// Reads a `REG_SZ` value; `None` when the key or value is missing.
#[cfg(windows)]
fn reg_query(key: &str, name: Option<&str>) -> Option<String> {
    let mut args = vec!["query", key];
    match name {
        Some(name) => args.extend(["/v", name]),
        None => args.push("/ve"),
    }
    let output = reg_run(&args)?;
    if !output.status.success() {
        return None;
    }
    parse_reg_value(&String::from_utf8_lossy(&output.stdout))
}

/// `reg query` prints `    <name>    REG_SZ    <value>`; the name is localized
/// for the default value, so only the type marker is reliable.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn parse_reg_value(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (_, value) = line.split_once("REG_SZ")?;
        Some(value.trim().to_string())
    })
}

/// Opens the system file manager with the file selected.
pub fn show_in_folder(path: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .args(["/select,", path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let parent = std::path::Path::new(path)
            .parent()
            .unwrap_or(std::path::Path::new("/"))
            .to_path_buf();
        std::process::Command::new("xdg-open")
            .arg(parent)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_reg_value;

    #[test]
    fn reads_reg_query_output() {
        let output = "\r\nHKEY_CURRENT_USER\\Software\\DwV\r\n    RegisteredExe    REG_SZ    C:\\Apps\\DwV\\dwv.exe\r\n\r\n";
        assert_eq!(
            parse_reg_value(output).as_deref(),
            Some("C:\\Apps\\DwV\\dwv.exe")
        );
        let default = "    (По умолчанию)    REG_SZ    \"C:\\qbittorrent.exe\" \"%1\"\r\n";
        assert_eq!(
            parse_reg_value(default).as_deref(),
            Some("\"C:\\qbittorrent.exe\" \"%1\"")
        );
        assert_eq!(parse_reg_value("ERROR: not found"), None);
    }
}
