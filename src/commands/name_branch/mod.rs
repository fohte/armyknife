mod backend;
mod error;

pub use backend::{Backend, detect_backend};
pub use error::{Error, Result};

use std::io::IsTerminal;

use clap::Args;
use indicatif::{ProgressBar, ProgressStyle};
use indoc::formatdoc;

/// Branch prefix for new branches
pub const BRANCH_PREFIX: &str = "fohte/";

#[derive(Args, Clone, PartialEq, Eq)]
pub struct NameBranchArgs {
    /// Task description to generate a branch name from
    pub description: String,
}

impl NameBranchArgs {
    pub fn run(&self) -> Result<()> {
        let backend = detect_backend();

        let spinner = if std::io::stderr().is_terminal() {
            let s = ProgressBar::new_spinner();
            #[allow(clippy::expect_used)] // static template string
            s.set_style(
                ProgressStyle::default_spinner()
                    .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ")
                    .template("{spinner} {msg}")
                    .expect("valid template"),
            );
            s.set_message("Generating branch name...");
            s.enable_steady_tick(std::time::Duration::from_millis(80));
            s
        } else {
            ProgressBar::hidden()
        };

        let name = generate_branch_name(&self.description, backend.as_ref());

        spinner.finish_and_clear();

        let name = name?;
        println!("{BRANCH_PREFIX}{name}");
        Ok(())
    }
}

/// Generate a branch name from a description using the specified backend.
///
/// Returns only the generated name part (without prefix).
/// The caller is responsible for adding the prefix if needed.
pub fn generate_branch_name(
    description: &str,
    backend: &(impl Backend + ?Sized),
) -> Result<String> {
    let prompt = build_prompt(description);
    let name = backend.generate(&prompt)?;
    let name = sanitize_branch_name(&name);
    validate_branch_name(&name)?;
    Ok(name)
}

fn build_prompt(description: &str) -> String {
    formatdoc! {r#"
        Task: Convert the following user task description to a git branch name.

        Requirements:
        - 2-4 words separated by hyphens (e.g., "fix-auth-timeout", "add-user-dashboard")
        - All lowercase
        - Use hyphens between words, never concatenate words

        <user-task-description>
        {description}
        </user-task-description>

        IMPORTANT: Output ONLY the branch name. Do not analyze, explain, or investigate the task. Just generate the name."#
    }
}

fn sanitize_branch_name(name: &str) -> String {
    let name = name.trim().to_lowercase();
    let mut result = String::with_capacity(name.len());
    let mut last_was_sep = true;

    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            result.push(ch);
            last_was_sep = false;
        } else if !last_was_sep {
            result.push('-');
            last_was_sep = true;
        }
    }

    if result.ends_with('-') {
        result.pop();
    }

    result
}

fn validate_branch_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::InvalidBranchName("generated name is empty".to_string()).into());
    }

    if !name
        .chars()
        .next()
        .map(|c| c.is_ascii_alphanumeric())
        .unwrap_or(false)
    {
        return Err(Error::InvalidBranchName(format!(
            "'{name}' must start with an alphanumeric character"
        ))
        .into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::simple("fix-login", "fix-login")]
    #[case::with_whitespace("  fix-login  ", "fix-login")]
    #[case::with_uppercase("Fix-Login", "fix-login")]
    #[case::with_invalid_chars("fix/login@bug", "fix-login-bug")]
    #[case::with_spaces("fix login bug", "fix-login-bug")]
    #[case::with_newline("fix-login\n", "fix-login")]
    #[case::leading_separator("/fix-login", "fix-login")]
    #[case::trailing_separator("fix-login/", "fix-login")]
    #[case::with_backticks("`fix-login`", "fix-login")]
    #[case::code_block_style("```\nfix-login\n```", "fix-login")]
    fn test_sanitize_branch_name(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(sanitize_branch_name(input), expected);
    }

    #[rstest]
    #[case::valid("fix-login", true)]
    #[case::valid_with_numbers("fix123", true)]
    #[case::empty("", false)]
    #[case::starts_with_dash("-fix", false)]
    fn test_validate_branch_name(#[case] name: &str, #[case] should_succeed: bool) {
        let result = validate_branch_name(name);
        assert_eq!(result.is_ok(), should_succeed);
    }
}
