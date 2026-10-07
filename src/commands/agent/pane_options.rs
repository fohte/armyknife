use crate::commands::agent::types::{Session, SessionStatus};
use crate::shared::config::{AgentConfig, AgentWorkTypeColor, AgentWorkTypeNamedColor};

pub(crate) const TMUX_CC_PANE_STATUS_OPTION: &str = "@armyknife-cc-pane-status";
pub(crate) const TMUX_CC_PANE_WORK_TYPE_OPTION: &str = "@armyknife-cc-pane-work-type";

pub(crate) fn tmux_option_commands(
    pane_id: &str,
    session: Option<&Session>,
    config: &AgentConfig,
) -> Vec<Vec<String>> {
    let status = session
        .filter(|session| session.status != SessionStatus::Ended)
        .map(|session| session.display_symbol().to_string());
    let work_type = session
        .and_then(|session| session.work_type.as_deref())
        .and_then(|work_type| config.work_type(work_type))
        .map(|config| format!("#[fg={}]{}", tmux_color(config.color), config.icon));

    vec![
        pane_option_command(pane_id, TMUX_CC_PANE_STATUS_OPTION, status.as_deref()),
        pane_option_command(pane_id, TMUX_CC_PANE_WORK_TYPE_OPTION, work_type.as_deref()),
    ]
}

fn tmux_color(color: AgentWorkTypeColor) -> String {
    match color {
        AgentWorkTypeColor::Named(AgentWorkTypeNamedColor::Reset) => "default".to_string(),
        AgentWorkTypeColor::Named(color) => match color.palette_index() {
            Some(index) => format!("colour{index}"),
            None => "default".to_string(),
        },
        AgentWorkTypeColor::Rgb([red, green, blue]) => format!("#{red:02x}{green:02x}{blue:02x}"),
        AgentWorkTypeColor::Indexed(index) => format!("colour{index}"),
    }
}

impl AgentWorkTypeNamedColor {
    fn palette_index(self) -> Option<u8> {
        match self {
            Self::Reset => None,
            Self::Black => Some(0),
            Self::Red => Some(1),
            Self::Green => Some(2),
            Self::Yellow => Some(3),
            Self::Blue => Some(4),
            Self::Magenta => Some(5),
            Self::Cyan => Some(6),
            Self::Gray => Some(7),
            Self::DarkGray => Some(8),
            Self::LightRed => Some(9),
            Self::LightGreen => Some(10),
            Self::LightYellow => Some(11),
            Self::LightBlue => Some(12),
            Self::LightMagenta => Some(13),
            Self::LightCyan => Some(14),
            Self::White => Some(15),
        }
    }
}

fn pane_option_command(pane_id: &str, option: &str, value: Option<&str>) -> Vec<String> {
    match value {
        Some(value) => vec![
            "set-option".to_string(),
            "-p".to_string(),
            "-t".to_string(),
            pane_id.to_string(),
            option.to_string(),
            value.to_string(),
        ],
        None => vec![
            "set-option".to_string(),
            "-p".to_string(),
            "-u".to_string(),
            "-t".to_string(),
            pane_id.to_string(),
            option.to_string(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    use chrono::Utc;
    use rstest::rstest;

    use super::*;
    use crate::commands::agent::types::Engine;
    use crate::shared::config::{AgentWorkTypeColor, AgentWorkTypeConfig};

    fn session(status: SessionStatus, work_type: Option<&str>) -> Session {
        Session {
            session_id: "session-test".to_string(),
            crit_urls: Vec::new(),
            pending_human_review_ids: BTreeSet::new(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            work_type: work_type.map(str::to_string),
            work_type_pinned: false,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: BTreeMap::new(),
            read_at: None,
            sweep_signaled: false,
            engine: Engine::Claude,
        }
    }

    fn config_with_work_type(color: AgentWorkTypeColor) -> AgentConfig {
        let mut config = AgentConfig::default();
        config.work_types.insert(
            "review".to_string(),
            AgentWorkTypeConfig {
                icon: "⚙".to_string(),
                color,
            },
        );
        config
    }

    #[test]
    fn tmux_options_use_session_display_and_configured_work_type() {
        assert_eq!(
            tmux_option_commands(
                "%42",
                Some(&session(SessionStatus::Running, Some("review"))),
                &config_with_work_type(AgentWorkTypeColor::Rgb([12, 34, 56])),
            ),
            vec![
                vec![
                    "set-option",
                    "-p",
                    "-t",
                    "%42",
                    "@armyknife-cc-pane-status",
                    "●",
                ],
                vec![
                    "set-option",
                    "-p",
                    "-t",
                    "%42",
                    "@armyknife-cc-pane-work-type",
                    "#[fg=#0c2238]⚙",
                ],
            ],
        );
    }

    #[rstest]
    #[case::reset(AgentWorkTypeColor::Named(AgentWorkTypeNamedColor::Reset), "default")]
    #[case::named(
        AgentWorkTypeColor::Named(AgentWorkTypeNamedColor::LightBlue),
        "colour12"
    )]
    #[case::rgb(AgentWorkTypeColor::Rgb([12, 34, 56]), "#0c2238")]
    #[case::indexed(AgentWorkTypeColor::Indexed(123), "colour123")]
    fn work_type_colors_use_tmux_syntax(
        #[case] color: AgentWorkTypeColor,
        #[case] expected_color: &str,
    ) {
        let expected_work_type = vec![
            "set-option".to_string(),
            "-p".to_string(),
            "-t".to_string(),
            "%42".to_string(),
            "@armyknife-cc-pane-work-type".to_string(),
            format!("#[fg={expected_color}]⚙"),
        ];
        assert_eq!(
            tmux_option_commands(
                "%42",
                Some(&session(SessionStatus::Running, Some("review"))),
                &config_with_work_type(color),
            ),
            vec![
                vec![
                    "set-option".to_string(),
                    "-p".to_string(),
                    "-t".to_string(),
                    "%42".to_string(),
                    "@armyknife-cc-pane-status".to_string(),
                    "●".to_string(),
                ],
                expected_work_type,
            ],
        );
    }

    #[rstest]
    #[case::ended(SessionStatus::Ended, Some("review"))]
    #[case::unknown_work_type(SessionStatus::Stopped, Some("missing"))]
    #[case::no_work_type(SessionStatus::Stopped, None)]
    fn tmux_options_unset_values_without_a_display(
        #[case] status: SessionStatus,
        #[case] work_type: Option<&str>,
    ) {
        let expected_status = if status == SessionStatus::Ended {
            vec![
                "set-option",
                "-p",
                "-u",
                "-t",
                "%42",
                "@armyknife-cc-pane-status",
            ]
        } else {
            vec![
                "set-option",
                "-p",
                "-t",
                "%42",
                "@armyknife-cc-pane-status",
                "✱",
            ]
        };
        let expected_work_type = if work_type == Some("review") {
            vec![
                "set-option",
                "-p",
                "-t",
                "%42",
                "@armyknife-cc-pane-work-type",
                "#[fg=#0c2238]⚙",
            ]
        } else {
            vec![
                "set-option",
                "-p",
                "-u",
                "-t",
                "%42",
                "@armyknife-cc-pane-work-type",
            ]
        };
        assert_eq!(
            tmux_option_commands(
                "%42",
                Some(&session(status, work_type)),
                &config_with_work_type(AgentWorkTypeColor::Rgb([12, 34, 56])),
            ),
            vec![expected_status, expected_work_type],
        );
    }

    #[test]
    fn tmux_options_unset_when_session_cannot_be_loaded() {
        assert_eq!(
            tmux_option_commands("%42", None, &AgentConfig::default()),
            vec![
                vec![
                    "set-option",
                    "-p",
                    "-u",
                    "-t",
                    "%42",
                    "@armyknife-cc-pane-status",
                ],
                vec![
                    "set-option",
                    "-p",
                    "-u",
                    "-t",
                    "%42",
                    "@armyknife-cc-pane-work-type",
                ],
            ],
        );
    }
}
