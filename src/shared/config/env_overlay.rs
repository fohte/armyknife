//! `ARMYKNIFE_*` environment variable overlay for `Config`.

use super::merge_yaml;

/// Build a YAML value from `ARMYKNIFE_*` environment variables, to be merged
/// on top of the YAML config files (highest priority).
///
/// The `ARMYKNIFE_` prefix is stripped, the rest is lowercased, and `__` splits
/// it into a dot-path of config keys (`_` alone isn't a valid separator since
/// keys themselves contain it, e.g. `auto_compact`). For example
/// `ARMYKNIFE_CC__AUTO_COMPACT__ENABLED=false` maps to `cc.auto_compact.enabled`.
/// Each value is parsed as a YAML scalar (so `false` is a bool, `3` a number).
///
/// `repos.*` entries are unreachable this way since repo keys contain `/`,
/// which can't appear in an environment variable name — that's fine, they're
/// out of scope for this overlay.
///
/// Variables whose path has no `__` are skipped rather than treated as an
/// unknown top-level key: every `Config` field is itself a struct, so a real
/// config path always needs at least one `__` to reach a leaf value. This
/// also keeps single-segment `ARMYKNIFE_*` variables that already have an
/// unrelated meaning (`ARMYKNIFE_SESSION_ID`, `ARMYKNIFE_WORKTREE_PATH`, etc.,
/// see `env_var.rs`) from being misread as config overrides.
///
/// Returns `None` if no `ARMYKNIFE_*` variable maps to a config path.
pub(super) fn env_overlay() -> Option<serde_yaml::Value> {
    const PREFIX: &str = "ARMYKNIFE_";

    let mut overlay: Option<serde_yaml::Value> = None;
    for (name, raw_value) in std::env::vars() {
        let Some(path) = name.strip_prefix(PREFIX) else {
            continue;
        };
        if !path.contains("__") {
            continue;
        }

        // A value like "Bottle: full" or "- 1" parses as a YAML mapping or
        // sequence rather than the literal string it's meant to be, and some
        // values aren't valid YAML at all. Env values are plain text, not
        // embedded documents, so only accept genuine scalars and otherwise
        // fall back to the raw string.
        let scalar = match serde_yaml::from_str::<serde_yaml::Value>(&raw_value) {
            Ok(
                value @ (serde_yaml::Value::Null
                | serde_yaml::Value::Bool(_)
                | serde_yaml::Value::Number(_)
                | serde_yaml::Value::String(_)),
            ) => value,
            _ => serde_yaml::Value::String(raw_value),
        };

        let mut node = scalar;
        for segment in path.to_ascii_lowercase().rsplit("__") {
            let mut mapping = serde_yaml::Mapping::new();
            mapping.insert(serde_yaml::Value::String(segment.to_string()), node);
            node = serde_yaml::Value::Mapping(mapping);
        }

        overlay = Some(match overlay {
            None => node,
            Some(base) => merge_yaml(base, node),
        });
    }

    overlay
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    /// Clears every ambient `ARMYKNIFE_*` variable that `env_overlay()` would
    /// pick up (path contains `__`) before applying `extra`, so this test
    /// isn't flaky depending on what's exported in the invoking shell —
    /// including this very feature's own overrides in a dev setup that
    /// dogfoods it.
    fn with_isolated_env_overlay<R>(extra: Vec<(&str, Option<&str>)>, f: impl FnOnce() -> R) -> R {
        let mut vars: Vec<(String, Option<String>)> = std::env::vars()
            .filter(|(k, _)| k.starts_with("ARMYKNIFE_") && k.contains("__"))
            .map(|(k, _)| (k, None))
            .collect();
        vars.extend(
            extra
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.map(str::to_string))),
        );
        temp_env::with_vars(vars, f)
    }

    #[rstest]
    #[case::bool_false("false", serde_yaml::Value::Bool(false))]
    #[case::number("3", serde_yaml::Value::Number(3.into()))]
    #[case::plain_string("hello", serde_yaml::Value::String("hello".to_string()))]
    // Regression: these all parse as a non-scalar YAML document (mapping or
    // sequence) if fed straight into serde_yaml, even though they're meant
    // as a literal string value.
    #[case::string_with_colon(
        "Bottle: full",
        serde_yaml::Value::String("Bottle: full".to_string())
    )]
    #[case::string_looks_like_sequence("- 1", serde_yaml::Value::String("- 1".to_string()))]
    #[case::string_looks_like_mapping("{a: 1}", serde_yaml::Value::String("{a: 1}".to_string()))]
    fn env_overlay_interprets_value_as_scalar_only(
        #[case] raw_value: &str,
        #[case] expected_scalar: serde_yaml::Value,
    ) {
        let overlay = with_isolated_env_overlay(
            vec![("ARMYKNIFE_NOTIFICATION__SOUND", Some(raw_value))],
            env_overlay,
        );

        let mut notification = serde_yaml::Mapping::new();
        notification.insert(
            serde_yaml::Value::String("sound".to_string()),
            expected_scalar,
        );
        let mut expected = serde_yaml::Mapping::new();
        expected.insert(
            serde_yaml::Value::String("notification".to_string()),
            serde_yaml::Value::Mapping(notification),
        );

        assert_eq!(overlay, Some(serde_yaml::Value::Mapping(expected)));
    }
}
