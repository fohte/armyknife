use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    pub login: String,
}

/// Trait for types that have an optional author field.
#[allow(dead_code)]
pub trait WithAuthor {
    fn author(&self) -> Option<&Author>;

    fn author_login(&self) -> &str {
        self.author().map(|a| a.login.as_str()).unwrap_or("unknown")
    }
}
