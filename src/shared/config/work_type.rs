use serde::{Deserialize, Serialize};

/// Display settings for a skill-backed agent work type.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentWorkTypeConfig {
    /// Icon shown for this work type.
    pub icon: String,

    /// Terminal color for this work type.
    pub color: AgentWorkTypeColor,
}

/// A terminal color accepted for an agent work type.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum AgentWorkTypeColor {
    Named(AgentWorkTypeNamedColor),
    Rgb([u8; 3]),
    Indexed(u8),
}

/// A named terminal color accepted for an agent work type.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentWorkTypeNamedColor {
    Reset,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    Gray,
    DarkGray,
    LightRed,
    LightGreen,
    LightYellow,
    LightBlue,
    LightMagenta,
    LightCyan,
    White,
}

impl From<AgentWorkTypeColor> for ratatui::style::Color {
    fn from(color: AgentWorkTypeColor) -> Self {
        match color {
            AgentWorkTypeColor::Named(color) => color.into(),
            AgentWorkTypeColor::Rgb([red, green, blue]) => Self::Rgb(red, green, blue),
            AgentWorkTypeColor::Indexed(index) => Self::Indexed(index),
        }
    }
}

impl From<AgentWorkTypeNamedColor> for ratatui::style::Color {
    fn from(color: AgentWorkTypeNamedColor) -> Self {
        match color {
            AgentWorkTypeNamedColor::Reset => Self::Reset,
            AgentWorkTypeNamedColor::Black => Self::Black,
            AgentWorkTypeNamedColor::Red => Self::Red,
            AgentWorkTypeNamedColor::Green => Self::Green,
            AgentWorkTypeNamedColor::Yellow => Self::Yellow,
            AgentWorkTypeNamedColor::Blue => Self::Blue,
            AgentWorkTypeNamedColor::Magenta => Self::Magenta,
            AgentWorkTypeNamedColor::Cyan => Self::Cyan,
            AgentWorkTypeNamedColor::Gray => Self::Gray,
            AgentWorkTypeNamedColor::DarkGray => Self::DarkGray,
            AgentWorkTypeNamedColor::LightRed => Self::LightRed,
            AgentWorkTypeNamedColor::LightGreen => Self::LightGreen,
            AgentWorkTypeNamedColor::LightYellow => Self::LightYellow,
            AgentWorkTypeNamedColor::LightBlue => Self::LightBlue,
            AgentWorkTypeNamedColor::LightMagenta => Self::LightMagenta,
            AgentWorkTypeNamedColor::LightCyan => Self::LightCyan,
            AgentWorkTypeNamedColor::White => Self::White,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::named(
        AgentWorkTypeColor::Named(AgentWorkTypeNamedColor::LightMagenta),
        ratatui::style::Color::LightMagenta
    )]
    #[case::rgb(AgentWorkTypeColor::Rgb([12, 34, 56]), ratatui::style::Color::Rgb(12, 34, 56))]
    #[case::indexed(AgentWorkTypeColor::Indexed(123), ratatui::style::Color::Indexed(123))]
    fn color_converts_to_ratatui_color(
        #[case] color: AgentWorkTypeColor,
        #[case] expected: ratatui::style::Color,
    ) {
        let actual: ratatui::style::Color = color.into();

        assert_eq!(actual, expected);
    }
}
