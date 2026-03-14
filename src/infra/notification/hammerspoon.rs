use std::process::Command;

use anyhow::{Context, Result, bail};

use super::types::Notification;

/// Sends a notification using Hammerspoon's `hs` CLI.
/// Click actions are handled via a pre-registered callback ("armyknife_notification")
/// in the Hammerspoon config. The command to execute on click is stored in a global
/// Lua table keyed by the notification's string representation.
pub fn send(notification: &Notification) -> Result<()> {
    let lua = build_send_lua(notification);

    let output = Command::new("hs")
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
    let lua = format!(
        "if _G._armyknife and _G._armyknife.remove_group then _G._armyknife.remove_group({}) end",
        lua_quote(group),
    );

    let output = Command::new("hs")
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
    parts.push("_G._armyknife = _G._armyknife or {commands = {}, groups = {}}".to_string());

    // Create notification with the registered callback tag if an action is present
    if notification.action().is_some() {
        parts.push("local n = hs.notify.new(\"armyknife_notification\")".to_string());
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

    // Store click action command keyed by notification object for the callback to retrieve
    if let Some(action) = notification.action() {
        parts.push(format!(
            "_G._armyknife.commands[tostring(n)] = {}",
            lua_quote(action.command()),
        ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::notification::NotificationAction;
    use rstest::rstest;

    #[rstest]
    #[case::simple("hello", "\"hello\"")]
    #[case::with_quotes("say \"hi\"", "\"say \\\"hi\\\"\"")]
    #[case::with_backslash("a\\b", "\"a\\\\b\"")]
    #[case::with_newline("line1\nline2", "\"line1\\nline2\"")]
    #[case::empty("", "\"\"")]
    fn test_lua_quote(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(lua_quote(input), expected);
    }

    #[test]
    fn test_build_send_lua_minimal() {
        let n = Notification::new("Title", "Body");
        let lua = build_send_lua(&n);
        let expected = concat!(
            "_G._armyknife = _G._armyknife or {commands = {}, groups = {}}; ",
            "local n = hs.notify.new(); ",
            "n:title(\"Title\"); ",
            "n:informativeText(\"Body\"); ",
            "n:withdrawAfter(0); ",
            "n:send()",
        );
        assert_eq!(lua, expected);
    }

    #[test]
    fn test_build_send_lua_with_all_fields() {
        let action = NotificationAction::new("tmux switch-client -t '%1'");
        let n = Notification::new("Done", "Task finished")
            .with_subtitle("sub")
            .with_sound("Glass")
            .with_action(action)
            .with_group("session-42")
            .with_app_icon("/tmp/icon.png");
        let lua = build_send_lua(&n);
        let expected = concat!(
            "_G._armyknife = _G._armyknife or {commands = {}, groups = {}}; ",
            "local n = hs.notify.new(\"armyknife_notification\"); ",
            "n:title(\"Done\"); ",
            "n:subTitle(\"sub\"); ",
            "n:informativeText(\"Task finished\"); ",
            "n:soundName(\"Glass\"); ",
            "_G._armyknife.commands[tostring(n)] = \"tmux switch-client -t '%1'\"; ",
            "_G._armyknife.groups[\"session-42\"] = _G._armyknife.groups[\"session-42\"] or {}; ",
            "table.insert(_G._armyknife.groups[\"session-42\"], n); ",
            "n:contentImage(hs.image.imageFromPath(\"/tmp/icon.png\")); ",
            "n:withdrawAfter(0); ",
            "n:send()",
        );
        assert_eq!(lua, expected);
    }
}
