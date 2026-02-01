# Gemini Code Assist Style Guide

## DO NOT suggest

- Changing versions or editions specified in config files. Assume version choices are intentional. Your training data may be outdated.
- Using `std::process::Command::new` outside of `src/infra/`. The infra module is the abstraction layer for external process invocation.
- Using `Result<T, E>` with custom error types for application-level functions. Use `anyhow::Result<T>` instead. (Domain-specific error types with `thiserror` are appropriate for library-level code.)
- Leaving dead code: unused functions, variables, imports, or unreachable code paths.
