//! `ARMYKNIFE_*` environment variable overlay for `Config`.

use super::merge_yaml;

/// Builds a YAML overlay from `ARMYKNIFE_*` env vars (highest priority),
/// merged on top of the YAML config files. Strips `ARMYKNIFE_`, lowercases
/// the rest, and splits on `__` (not `_`, since keys like `auto_compact`
/// contain it) into a config dot-path — e.g.
/// `ARMYKNIFE_AGENT__AUTO_COMPACT__ENABLED=false` maps to
/// `agent.auto_compact.enabled`. Legacy `WM` and `CC` paths are normalized to
/// their `agent` paths. Values parse as YAML scalars.
///
/// Paths without `__` are skipped, since every `Config` field is a struct
/// and no real override is single-segment; this also avoids misreading
/// unrelated vars like `ARMYKNIFE_SESSION_ID` (see `env_var.rs`) as config
/// keys. `repos.*` and `orgs.*` stay unreachable too, since repo keys
/// contain `/` and org logins can't be safely case-folded.
/// Returns `None` when no variable maps to a config path.
pub(super) fn env_overlay() -> Option<serde_yaml::Value> {
    env_overlay_from(std::env::vars())
}

pub(super) fn env_overlay_from(
    vars: impl IntoIterator<Item = (String, String)>,
) -> Option<serde_yaml::Value> {
    const PREFIX: &str = "ARMYKNIFE_";

    let mut legacy_overlay: Option<serde_yaml::Value> = None;
    let mut current_overlay: Option<serde_yaml::Value> = None;
    for (name, raw_value) in vars {
        let Some(path) = name.strip_prefix(PREFIX) else {
            continue;
        };
        if !path.contains("__") {
            continue;
        }
        // `orgs.*` keys are GitHub org logins, not struct field names, so
        // lowercasing them below (like the rest of the path) could silently
        // create a separate entry instead of overriding the intended org.
        if path
            .split("__")
            .next()
            .is_some_and(|s| s.eq_ignore_ascii_case("orgs"))
        {
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

        let (path, is_legacy) = normalize_legacy_path(path);
        let mut node = scalar;
        for segment in path.rsplit("__") {
            let mut mapping = serde_yaml::Mapping::new();
            mapping.insert(serde_yaml::Value::String(segment.to_string()), node);
            node = serde_yaml::Value::Mapping(mapping);
        }

        let overlay = if is_legacy {
            &mut legacy_overlay
        } else {
            &mut current_overlay
        };
        *overlay = Some(match overlay.take() {
            None => node,
            Some(base) => merge_yaml(base, node),
        });
    }

    match (legacy_overlay, current_overlay) {
        (Some(legacy), Some(current)) => Some(merge_yaml(legacy, current)),
        (Some(legacy), None) => Some(legacy),
        (None, current) => current,
    }
}

fn normalize_legacy_path(path: &str) -> (String, bool) {
    let path = path.to_ascii_lowercase();
    if let Some(suffix) = path.strip_prefix("wm__") {
        let is_legacy_worktrees_dir =
            suffix == "worktrees_dir" || suffix.starts_with("worktrees_dir__");
        if ["branch_prefix", "layout", "repos_root"]
            .iter()
            .any(|field| suffix == *field || suffix.starts_with(&format!("{field}__")))
            || is_legacy_worktrees_dir
        {
            let suffix = if is_legacy_worktrees_dir {
                suffix.replacen("worktrees_dir", "dir", 1)
            } else {
                suffix.to_string()
            };
            return (format!("agent__worktree__{suffix}"), true);
        }
        return (path, true);
    }

    for field in ["auto_pause", "auto_compact"] {
        let legacy_prefix = format!("cc__{field}");
        if path == legacy_prefix || path.starts_with(&format!("{legacy_prefix}__")) {
            return (
                format!("agent__{field}{}", &path[legacy_prefix.len()..]),
                true,
            );
        }
    }

    (path, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn overlay_from(vars: &[(&str, &str)]) -> Option<serde_yaml::Value> {
        env_overlay_from(
            vars.iter()
                .map(|(name, value)| (name.to_string(), value.to_string())),
        )
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
        let overlay = overlay_from(&[("ARMYKNIFE_NOTIFICATION__SOUND", raw_value)]);

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

    #[rstest]
    #[case::lowercase("ARMYKNIFE_ORGS__SOMEORG__AI__REVIEW__REVIEWERS")]
    #[case::mixed_case("ARMYKNIFE_Orgs__SomeOrg__AI__REVIEW__REVIEWERS")]
    fn env_overlay_skips_orgs_paths(#[case] var_name: &str) {
        let overlay = overlay_from(&[(var_name, "devin")]);

        assert_eq!(overlay, None);
    }
}
