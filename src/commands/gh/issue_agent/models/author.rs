use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Author {
    pub login: String,
}

/// Trait for types that have an optional author field.
pub trait WithAuthor {
    fn author(&self) -> Option<&Author>;

    fn author_login(&self) -> &str {
        self.author().map(|a| a.login.as_str()).unwrap_or("unknown")
    }
}
