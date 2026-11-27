use clap::Args;

use crate::infra::process::ProcessSnapshot;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct CurrentArgs {}

pub fn run() -> i32 {
    let detected_by_environment = std::env::var_os("CLAUDECODE").is_some();
    let process_engine = if detected_by_environment {
        None
    } else {
        ProcessSnapshot::capture()
            .and_then(|snapshot| snapshot.agent_ancestor_engine(std::process::id()))
    };

    match output_for(detected_by_environment, process_engine) {
        Some(output) => {
            println!("{output}");
            0
        }
        None => 1,
    }
}

fn output_for(detected_by_environment: bool, process_engine: Option<&str>) -> Option<String> {
    let engine = if detected_by_environment {
        Some("claude-code")
    } else {
        process_engine
    }?;

    Some(format!(r#"{{"engine":"{engine}"}}"#))
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::output_for;

    #[rstest]
    #[case::claude_code_environment(true, None, Some(r#"{"engine":"claude-code"}"#))]
    #[case::codex_ancestor(false, Some("codex"), Some(r#"{"engine":"codex"}"#))]
    #[case::outside_agent(false, None, None)]
    fn current_output_cases(
        #[case] detected_by_environment: bool,
        #[case] process_engine: Option<&str>,
        #[case] expected: Option<&str>,
    ) {
        assert_eq!(
            output_for(detected_by_environment, process_engine).as_deref(),
            expected
        );
    }
}
