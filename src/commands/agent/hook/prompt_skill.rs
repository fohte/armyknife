use super::super::types::Engine;

/// Finds an explicit leading skill command; Codex work-type inference scans later mentions too.
pub(super) fn leading_prompt_skill(engine: Engine, prompt: &str) -> Option<&str> {
    let first_token = prompt.split_whitespace().next()?;

    match engine {
        Engine::Claude => first_token.strip_prefix('/'),
        Engine::Codex => first_token.strip_prefix('$'),
    }
}
