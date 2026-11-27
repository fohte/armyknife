use clap::Args;

use crate::infra::process::ProcessSnapshot;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct CurrentArgs {}

#[derive(Debug, PartialEq, Eq)]
pub struct CurrentCommandOutput {
    pub stdout: Option<String>,
    pub exit_code: i32,
}

pub trait CurrentAgentContext {
    fn process_engine(&self) -> Option<&'static str>;
    fn claude_code_env_is_set(&self) -> bool;
}

pub struct ProcessContext;

impl CurrentAgentContext for ProcessContext {
    fn process_engine(&self) -> Option<&'static str> {
        ProcessSnapshot::capture()
            .and_then(|snapshot| snapshot.agent_ancestor_engine(std::process::id()))
    }

    fn claude_code_env_is_set(&self) -> bool {
        std::env::var_os("CLAUDECODE").is_some()
    }
}

pub fn run(context: &impl CurrentAgentContext) -> CurrentCommandOutput {
    let engine = context
        .process_engine()
        .or_else(|| context.claude_code_env_is_set().then_some("claude-code"));

    match engine {
        Some(engine) => CurrentCommandOutput {
            stdout: Some(format!(r#"{{"engine":"{engine}"}}"#)),
            exit_code: 0,
        },
        None => CurrentCommandOutput {
            stdout: None,
            exit_code: 1,
        },
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{CurrentAgentContext, CurrentCommandOutput, run};

    struct FakeContext {
        process_engine: Option<&'static str>,
        claude_code_env_is_set: bool,
    }

    impl CurrentAgentContext for FakeContext {
        fn process_engine(&self) -> Option<&'static str> {
            self.process_engine
        }

        fn claude_code_env_is_set(&self) -> bool {
            self.claude_code_env_is_set
        }
    }

    #[rstest]
    #[case::claude_code_environment_fallback(
        None,
        true,
        CurrentCommandOutput {
            stdout: Some(r#"{"engine":"claude-code"}"#.to_string()),
            exit_code: 0,
        },
    )]
    #[case::nearest_codex_ancestor_overrides_environment(
        Some("codex"),
        true,
        CurrentCommandOutput {
            stdout: Some(r#"{"engine":"codex"}"#.to_string()),
            exit_code: 0,
        },
    )]
    #[case::codex_ancestor(
        Some("codex"),
        false,
        CurrentCommandOutput {
            stdout: Some(r#"{"engine":"codex"}"#.to_string()),
            exit_code: 0,
        },
    )]
    #[case::outside_agent(
        None,
        false,
        CurrentCommandOutput {
            stdout: None,
            exit_code: 1,
        },
    )]
    fn current_command_cases(
        #[case] process_engine: Option<&'static str>,
        #[case] claude_code_env_is_set: bool,
        #[case] expected: CurrentCommandOutput,
    ) {
        let context = FakeContext {
            process_engine,
            claude_code_env_is_set,
        };

        assert_eq!(run(&context), expected);
    }
}
