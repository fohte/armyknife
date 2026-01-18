//! Test factories for creating test data with sensible defaults.
//!
//! This module provides factory functions for creating test objects.
//! Use `*_with()` variants to customize specific fields.
//!
//! # Example
//! ```ignore
//! use crate::testing::factories::{issue, issue_with, comment};
//!
//! // Create with defaults
//! let i = issue();
//!
//! // Customize specific fields
//! let i = issue_with(|i| {
//!     i.title = "Custom Title".to_string();
//!     i.number = 42;
//! });
//! ```

use chrono::{Duration, Utc};

use crate::gh::issue_agent::models::{Author, Comment, Issue, Label};

// =============================================================================
// Issue factories
// =============================================================================

/// Create an Issue with default test values.
pub fn issue() -> Issue {
    Issue {
        number: 1,
        title: "Test Issue".to_string(),
        body: Some("Test body".to_string()),
        state: "OPEN".to_string(),
        labels: vec![],
        assignees: vec![],
        milestone: None,
        author: Some(Author {
            login: "testuser".to_string(),
        }),
        // Use relative times for consistent relative time formatting in tests
        created_at: Utc::now() - Duration::hours(2),
        updated_at: Utc::now(),
    }
}

/// Create an Issue with customizations applied via closure.
pub fn issue_with(f: impl FnOnce(&mut Issue)) -> Issue {
    let mut i = issue();
    f(&mut i);
    i
}

// =============================================================================
// Comment factories (for gh/issue_agent)
// =============================================================================

/// Create a Comment with default test values.
pub fn comment() -> Comment {
    Comment {
        id: "IC_123".to_string(),
        database_id: 123,
        author: Some(Author {
            login: "commenter".to_string(),
        }),
        created_at: Utc::now() - Duration::hours(1),
        body: "Test comment".to_string(),
    }
}

/// Create a Comment with customizations applied via closure.
pub fn comment_with(f: impl FnOnce(&mut Comment)) -> Comment {
    let mut c = comment();
    f(&mut c);
    c
}

// =============================================================================
// Helper factories
// =============================================================================

/// Create an Author with the given login.
pub fn author(login: &str) -> Author {
    Author {
        login: login.to_string(),
    }
}

/// Create a Label with the given name.
pub fn label(name: &str) -> Label {
    Label {
        name: name.to_string(),
    }
}

/// Create multiple labels from a slice of names.
pub fn labels(names: &[&str]) -> Vec<Label> {
    names.iter().map(|n| label(n)).collect()
}

/// Create multiple authors (assignees) from a slice of logins.
pub fn assignees(logins: &[&str]) -> Vec<Author> {
    logins.iter().map(|l| author(l)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_issue_defaults() {
        let i = issue();
        assert_eq!(i.number, 1);
        assert_eq!(i.title, "Test Issue");
        assert_eq!(i.state, "OPEN");
    }

    #[test]
    fn test_issue_with_customization() {
        let i = issue_with(|i| {
            i.number = 42;
            i.title = "Custom".to_string();
            i.labels = labels(&["bug", "urgent"]);
        });
        assert_eq!(i.number, 42);
        assert_eq!(i.title, "Custom");
        assert_eq!(i.labels.len(), 2);
    }

    #[test]
    fn test_comment_defaults() {
        let c = comment();
        assert_eq!(c.id, "IC_123");
        assert_eq!(c.body, "Test comment");
    }
}
