pub(crate) fn is_enabled(config_enabled: bool, cc_notify: Option<&str>) -> bool {
    match cc_notify {
        Some(value) => !matches!(value.to_lowercase().as_str(), "0" | "false"),
        None => config_enabled,
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::enabled_by_config(true, None, true)]
    #[case::disabled_by_config(false, None, false)]
    #[case::disabled_by_zero_override(true, Some("0"), false)]
    #[case::disabled_by_false_override(true, Some("false"), false)]
    #[case::disabled_by_case_insensitive_false_override(true, Some("FALSE"), false)]
    #[case::enabled_by_override(false, Some("1"), true)]
    fn honors_config_and_environment_override(
        #[case] config_enabled: bool,
        #[case] cc_notify: Option<&str>,
        #[case] expected: bool,
    ) {
        assert_eq!(is_enabled(config_enabled, cc_notify), expected);
    }
}
