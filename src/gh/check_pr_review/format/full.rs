use super::color::{BG_GRAY, DIM, RESET};
use super::{FormatOptions, author_login, format_datetime, process_body, state_indicator};
use crate::gh::check_pr_review::models::{Comment, PrData, Review, ReviewThread};
use crate::gh::check_pr_review::{CheckPrReviewError, Result};

pub fn print_full(pr_data: &PrData, options: &FormatOptions) {
    let sorted_reviews = pr_data.sorted_reviews();

    for review in sorted_reviews {
        print_review_with_threads(review, pr_data, options);
    }

    let orphan_threads = pr_data.orphan_threads();
    for thread in orphan_threads {
        print_thread(thread, options);
    }
}

pub fn print_review_details(
    pr_data: &PrData,
    review_num: usize,
    options: &FormatOptions,
) -> Result<()> {
    if review_num == 0 {
        return Err(CheckPrReviewError::ReviewNotFound(review_num));
    }

    let sorted_reviews = pr_data.sorted_reviews();

    let review = sorted_reviews
        .get(review_num - 1)
        .ok_or(CheckPrReviewError::ReviewNotFound(review_num))?;

    print_review_with_threads(review, pr_data, options);
    Ok(())
}

fn print_review_with_threads(review: &Review, pr_data: &PrData, options: &FormatOptions) {
    print_review(review, options);

    let review_threads = pr_data.threads_for_review(review.database_id);
    for thread in review_threads {
        print_thread(thread, options);
    }
}

fn print_review(review: &Review, options: &FormatOptions) {
    let formatted_date = format_datetime(&review.created_at);
    println!(
        "{BG_GRAY} @{} ({formatted_date}) {RESET} {}",
        author_login(review),
        state_indicator(review.state)
    );

    let body = process_body(&review.body, options);
    if !body.is_empty() {
        println!("{body}");
    }
    println!();
}

fn print_thread(thread: &ReviewThread, options: &FormatOptions) {
    if let Some(root) = thread.root_comment() {
        print_comment(root, "", false, thread.is_resolved, options);

        for reply in thread.replies() {
            print_comment(reply, "  ", true, thread.is_resolved, options);
        }
    }
}

fn print_comment(
    comment: &Comment,
    indent: &str,
    is_reply: bool,
    is_resolved: bool,
    options: &FormatOptions,
) {
    let formatted_date = format_datetime(&comment.created_at);

    if is_reply {
        println!(
            "{indent}└─ {BG_GRAY} @{} ({formatted_date}) {RESET}",
            comment.author_login()
        );
    } else {
        let resolved_indicator = if is_resolved {
            format!(" {DIM}[resolved]{RESET}")
        } else {
            String::new()
        };
        println!(
            "{BG_GRAY} @{} ({formatted_date}) {RESET}{resolved_indicator}",
            comment.author_login()
        );
    }

    if !is_reply && let Some(diff_hunk) = &comment.diff_hunk {
        let path = comment.path.as_deref().unwrap_or("?");
        println!("File: {path}");
        let lines: Vec<&str> = diff_hunk.lines().collect();
        let start = lines.len().saturating_sub(3);
        for line in &lines[start..] {
            println!("{line}");
        }
    }

    let body = process_body(&comment.body, options);
    if is_reply {
        for line in body.lines() {
            println!("{indent}   {line}");
        }
    } else {
        println!("{body}");
    }
    println!();
}
