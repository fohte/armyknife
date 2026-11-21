use std::collections::HashMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct FoldedTodoSummary {
    pub(super) count: usize,
    pub(super) first_title: Option<String>,
}

pub(super) fn summarize(
    todos: impl IntoIterator<Item = (String, String, bool)>,
) -> (HashMap<String, String>, HashMap<String, FoldedTodoSummary>) {
    let mut unanswered_titles = HashMap::new();
    let mut folded_summaries = HashMap::<String, FoldedTodoSummary>::new();
    for (session_id, title, folded) in todos {
        if folded {
            let summary = folded_summaries.entry(session_id).or_default();
            summary.count += 1;
            summary.first_title.get_or_insert(title);
        } else {
            unanswered_titles.entry(session_id).or_insert(title);
        }
    }
    (unanswered_titles, folded_summaries)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{FoldedTodoSummary, summarize};

    #[test]
    fn separates_open_todos_from_folded_later_summaries() {
        let result = summarize([
            ("session-b".to_string(), "Check later".to_string(), true),
            ("session-a".to_string(), "Do now".to_string(), false),
            (
                "session-b".to_string(),
                "Check another thing".to_string(),
                true,
            ),
            ("session-a".to_string(), "Also later".to_string(), true),
        ]);

        assert_eq!(
            result,
            (
                HashMap::from([("session-a".to_string(), "Do now".to_string())]),
                HashMap::from([
                    (
                        "session-a".to_string(),
                        FoldedTodoSummary {
                            count: 1,
                            first_title: Some("Also later".to_string()),
                        },
                    ),
                    (
                        "session-b".to_string(),
                        FoldedTodoSummary {
                            count: 2,
                            first_title: Some("Check later".to_string()),
                        },
                    ),
                ]),
            ),
        );
    }
}
