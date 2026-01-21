use super::{author_login, truncate_text};
use crate::commands::gh::check_pr_review::models::{PrData, ReviewThread};

pub fn print_summary(pr_data: &PrData) {
    print!("{}", format_summary(pr_data));
}

pub fn format_summary(pr_data: &PrData) -> String {
    let mut output = String::new();
    let sorted_reviews = pr_data.sorted_reviews();

    for (index, review) in sorted_reviews.iter().enumerate() {
        let review_num = index + 1;
        let review_threads = pr_data.threads_for_review(review.database_id);
        let thread_count = review_threads.len();
        let unresolved_count = ReviewThread::count_unresolved(&review_threads);

        let thread_info = if thread_count > 0 {
            format!(" - {unresolved_count}/{thread_count} unresolved")
        } else {
            String::new()
        };

        output.push_str(&format!(
            "[{}] @{} ({}){thread_info}\n",
            review_num,
            author_login(review),
            review.state.as_str()
        ));

        if !review.body.is_empty() {
            let body_preview = truncate_text(&review.body, 70);
            output.push_str(&format!("    \"{body_preview}\"\n"));
        }

        for thread in &review_threads {
            output.push_str(&format_thread_summary(thread));
        }

        output.push('\n');
    }

    let orphan_threads = pr_data.orphan_threads();
    if !orphan_threads.is_empty() {
        output.push_str(&format!(
            "Orphan threads (not associated with a review): {}\n",
            orphan_threads.len()
        ));
        for thread in orphan_threads {
            output.push_str(&format_thread_summary(thread));
        }
    }

    output
}

fn format_thread_summary(thread: &ReviewThread) -> String {
    let mut output = String::new();
    if let Some(root) = thread.root_comment() {
        let path = root.path.as_deref().unwrap_or("?");
        let line = root
            .effective_line()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "?".to_string());
        let comment_count = thread.comments.nodes.len();

        let resolved_mark = if thread.is_resolved {
            " [resolved]"
        } else {
            ""
        };

        let body_preview = truncate_text(&root.body, 50);
        output.push_str(&format!(
            "    - {path}:{line} ({comment_count} comments){resolved_mark}\n"
        ));
        output.push_str(&format!(
            "      @{}: \"{body_preview}\"\n",
            root.author_login()
        ));
    }
    output
}
