use super::{author_login, truncate_text};
use crate::gh::check_pr_review::models::{PrData, ReviewThread};

pub fn print_summary(pr_data: &PrData) {
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

        println!(
            "[{}] @{} ({}){thread_info}",
            review_num,
            author_login(review),
            review.state.as_str()
        );

        if !review.body.is_empty() {
            let body_preview = truncate_text(&review.body, 70);
            println!("    \"{body_preview}\"");
        }

        for thread in &review_threads {
            print_thread_summary(thread);
        }

        println!();
    }

    let orphan_threads = pr_data.orphan_threads();
    if !orphan_threads.is_empty() {
        println!(
            "Orphan threads (not associated with a review): {}",
            orphan_threads.len()
        );
        for thread in orphan_threads {
            print_thread_summary(thread);
        }
    }
}

fn print_thread_summary(thread: &ReviewThread) {
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
        println!("    - {path}:{line} ({comment_count} comments){resolved_mark}");
        println!("      @{}: \"{body_preview}\"", root.author_login());
    }
}
