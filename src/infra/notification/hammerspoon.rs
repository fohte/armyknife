use std::io;
use std::process::{Command, Output};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

use crate::infra::external_tool::ExternalTool;
use crate::infra::process;

use super::types::Notification;

/// Bounds the hs process itself because its `-t` option only limits IPC with Hammerspoon.
const HAMMERSPOON_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// Sends a notification using Hammerspoon's `hs` CLI.
/// Click actions are handled via a pre-registered callback ("armyknife_notification")
/// in the Hammerspoon config. The command to execute on click is stored in a global
/// Lua table keyed by the notification's string representation.
pub fn send(notification: &Notification) -> Result<()> {
    let lua = build_send_lua(notification);
    run_hs(&lua, "hs notification failed")
}

/// Removes notifications belonging to the given group.
/// Delegates to a Lua helper defined in the Hammerspoon config.
pub fn remove_group(group: &str) -> Result<()> {
    let g = lua_quote(group);
    let lua = format!(
        "if _G._armyknife and _G._armyknife.groups and _G._armyknife.groups[{g}] then for _, n in ipairs(_G._armyknife.groups[{g}]) do n:withdraw() end; _G._armyknife.groups[{g}] = nil end"
    );
    run_hs(&lua, "hs remove_group failed")
}

fn run_hs(lua: &str, fail_label: &str) -> Result<()> {
    if !ExternalTool::Hammerspoon.is_available() {
        bail!("hs command not found");
    }
    run_hs_with(
        ExternalTool::Hammerspoon.command(),
        lua,
        fail_label,
        process::run_with_timeout,
    )
}

fn run_hs_with(
    mut command: Command,
    lua: &str,
    fail_label: &str,
    run: impl FnOnce(Command, Duration) -> io::Result<Output>,
) -> Result<()> {
    command.arg("-c").arg(lua);
    let output =
        run(command, HAMMERSPOON_COMMAND_TIMEOUT).context("failed to execute hs command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{fail_label}: {stderr}");
    }
    Ok(())
}

/// Builds the Lua code string to create and send a Hammerspoon notification.
fn build_send_lua(notification: &Notification) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Ensure the global armyknife namespace exists
    parts.push(
        "_G._armyknife = _G._armyknife or {}; _G._armyknife.groups = _G._armyknife.groups or {}"
            .to_string(),
    );

    // For click actions, register a per-notification callback with a unique tag.
    // The callback includes the command directly as a closure, avoiding the need
    // to correlate notification objects across IPC boundaries.
    if let Some(action) = notification.action() {
        let tag = generate_tag();
        let current_path = std::env::var("PATH").unwrap_or_default();
        let current_home = std::env::var("HOME").unwrap_or_default();
        parts.push(format!("local tag = {}", lua_quote(&tag)));
        // Capture PATH and HOME at notification creation time so click callbacks
        // can find commands and resolve cache/config directories. hs.task.new
        // provides only a minimal environment by default.
        parts.push(format!(
            "hs.notify.register(tag, function() local t = hs.task.new(\"/bin/sh\", function() end, {{\"-c\", {}}}); t:setEnvironment({{PATH = {}, HOME = {}}}); t:start() end)",
            lua_quote(action.command()),
            lua_quote(&current_path),
            lua_quote(&current_home),
        ));
        parts.push("local n = hs.notify.new(tag)".to_string());
        parts.push("n:hasActionButton(true)".to_string());
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

    // Bound image loading so an unreachable URL cannot delay notification delivery indefinitely.
    let send_notification = if let Some(app_icon) = notification.app_icon() {
        parts.push(format!(
            "n:contentImage(hs.image.imageFromPath({}))",
            lua_quote(app_icon)
        ));
        None
    } else {
        notification.content_image_url().map(|content_image_url| {
            format!(
                "local sent = false; local send_notification = function(content_image) if not sent then sent = true; if content_image then pcall(function() n:contentImage(content_image) end) end; n:send() end end; local image_timeout = hs.timer.doAfter(1, function() send_notification(nil) end); hs.image.imageFromURL({}, function(content_image) if not sent then image_timeout:stop(); send_notification(content_image) end end)",
                lua_quote(content_image_url),
            )
        })
    };

    // Disable auto-withdraw so the notification stays until clicked or explicitly removed
    parts.push("n:withdrawAfter(0)".to_string());

    if let Some(send_notification) = send_notification {
        parts.push(send_notification);
    } else {
        parts.push("n:send()".to_string());
    }

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

#[cfg(test)]
mod tests {
    use std::io;
    use std::process::Command;
    use std::time::Duration;

    use super::{build_send_lua, run_hs_with};
    use crate::infra::notification::Notification;

    #[test]
    fn attaches_async_content_image_before_sending_once() {
        let notification = Notification::new("Review", "sample message")
            .with_content_image_url("https://images.example.test/mark.png");

        assert_eq!(
            build_send_lua(&notification),
            "_G._armyknife = _G._armyknife or {}; _G._armyknife.groups = _G._armyknife.groups or {}; local n = hs.notify.new(); n:title(\"Review\"); n:informativeText(\"sample message\"); n:withdrawAfter(0); local sent = false; local send_notification = function(content_image) if not sent then sent = true; if content_image then pcall(function() n:contentImage(content_image) end) end; n:send() end end; local image_timeout = hs.timer.doAfter(1, function() send_notification(nil) end); hs.image.imageFromURL(\"https://images.example.test/mark.png\", function(content_image) if not sent then image_timeout:stop(); send_notification(content_image) end end)",
        );
    }

    #[test]
    fn hs_command_timeout_is_reported_as_an_execution_error() {
        let mut observed = (None, Vec::new());
        let result = run_hs_with(
            Command::new("hs"),
            "return 1",
            "hs remove_group failed",
            |command, timeout| {
                observed = (
                    Some(timeout),
                    command
                        .get_args()
                        .map(|arg| arg.to_string_lossy().into_owned())
                        .collect(),
                );
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "command timed out after 5s",
                ))
            },
        );

        assert_eq!(
            (
                observed,
                format!(
                    "{:#}",
                    result.expect_err("timeout should be returned as an error")
                ),
            ),
            (
                (
                    Some(Duration::from_secs(5)),
                    vec!["-c".to_string(), "return 1".to_string()],
                ),
                "failed to execute hs command: command timed out after 5s".to_string(),
            ),
        );
    }
}
