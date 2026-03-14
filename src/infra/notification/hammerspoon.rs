use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

use crate::shared::command::find_command_path;

use super::types::Notification;

/// Well-known location of the `hs` CLI bundled inside Hammerspoon.app.
const HS_BUNDLED_PATH: &str = "/Applications/Hammerspoon.app/Contents/Frameworks/hs/hs";

/// Returns the path to the `hs` CLI, checking PATH first then the bundled location.
pub fn find_hs_path() -> Option<PathBuf> {
    if let Some(p) = find_command_path("hs") {
        return Some(p);
    }
    let bundled = PathBuf::from(HS_BUNDLED_PATH);
    if bundled.is_file() {
        return Some(bundled);
    }
    None
}

/// Sends a notification using Hammerspoon's `hs` CLI.
/// Click actions are handled via a pre-registered callback ("armyknife_notification")
/// in the Hammerspoon config. The command to execute on click is stored in a global
/// Lua table keyed by the notification's string representation.
pub fn send(notification: &Notification) -> Result<()> {
    let hs = find_hs_path().context("hs command not found")?;
    let lua = build_send_lua(notification);

    let output = Command::new(&hs)
        .arg("-c")
        .arg(&lua)
        .output()
        .context("failed to execute hs command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("hs notification failed: {}", stderr);
    }

    Ok(())
}

/// Removes notifications belonging to the given group.
/// Delegates to a Lua helper defined in the Hammerspoon config.
pub fn remove_group(group: &str) -> Result<()> {
    let hs = find_hs_path().context("hs command not found")?;
    let lua = format!(
        "if _G._armyknife and _G._armyknife.remove_group then _G._armyknife.remove_group({}) end",
        lua_quote(group),
    );

    let output = Command::new(&hs)
        .arg("-c")
        .arg(&lua)
        .output()
        .context("failed to execute hs -c for remove_group")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("hs remove_group failed: {}", stderr);
    }

    Ok(())
}

/// Builds the Lua code string to create and send a Hammerspoon notification.
fn build_send_lua(notification: &Notification) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Ensure the global armyknife namespace exists
    parts.push("_G._armyknife = _G._armyknife or {groups = {}}".to_string());

    // For click actions, register a per-notification callback with a unique tag.
    // The callback includes the command directly as a closure, avoiding the need
    // to correlate notification objects across IPC boundaries.
    if let Some(action) = notification.action() {
        let tag = generate_tag();
        parts.push(format!("local tag = {}", lua_quote(&tag),));
        parts.push(format!(
            "hs.notify.register(tag, function() hs.execute({}, true) end)",
            lua_quote(action.command()),
        ));
        parts.push("local n = hs.notify.new(tag)".to_string());
    } else {
        parts.push("local n = hs.notify.new()".to_string());
    }

    parts.push(format!("n:title({})", lua_quote(notification.title())));

    if let Some(subtitle) = notification.subtitle() {
        parts.push(format!("n:subTitle({})", lua_quote(subtitle)));
    }

    parts.push(format!(
        "n:informativeText({})",
        lua_quote(notification.message())
    ));

    if let Some(sound) = notification.sound() {
        parts.push(format!("n:soundName({})", lua_quote(sound)));
    }

    // Store notification by group for later withdrawal
    if let Some(group) = notification.group() {
        parts.push(format!(
            "_G._armyknife.groups[{g}] = _G._armyknife.groups[{g}] or {{}}; table.insert(_G._armyknife.groups[{g}], n)",
            g = lua_quote(group),
        ));
    }

    if let Some(app_icon) = notification.app_icon() {
        parts.push(format!(
            "n:contentImage(hs.image.imageFromPath({}))",
            lua_quote(app_icon)
        ));
    }

    // Disable auto-withdraw so the notification stays until clicked or explicitly removed
    parts.push("n:withdrawAfter(0)".to_string());
    parts.push("n:send()".to_string());

    parts.join("; ")
}

/// Generates a unique tag for a notification callback.
fn generate_tag() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("armyknife_{ts}")
}

/// Escapes a string for use as a Lua string literal (double-quoted).
fn lua_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}
