use super::color::{BG_GRAY, DIM, RESET};
use super::{
    FormatOptions, author_login, format_datetime, format_diff_with_delta, process_body,
    state_indicator,
};
use crate::gh::check_pr_review::models::{Comment, PrData, Review, ReviewThread};
use crate::gh::check_pr_review::{CheckPrReviewError, Result};

pub fn print_full(pr_data: &PrData, options: &FormatOptions) {
    print!("{}", format_full(pr_data, options));
}

pub fn format_full(pr_data: &PrData, options: &FormatOptions) -> String {
    let mut output = String::new();
    let sorted_reviews = pr_data.sorted_reviews();

    for review in sorted_reviews {
        output.push_str(&format_review_with_threads(review, pr_data, options));
    }

    let orphan_threads = pr_data.orphan_threads();
    for thread in orphan_threads {
        output.push_str(&format_thread(thread, options));
    }

    output
}

pub fn print_review_details(
    pr_data: &PrData,
    review_num: usize,
    options: &FormatOptions,
) -> Result<()> {
    print!("{}", format_review_details(pr_data, review_num, options)?);
    Ok(())
}

pub fn format_review_details(
    pr_data: &PrData,
    review_num: usize,
    options: &FormatOptions,
) -> Result<String> {
    if review_num == 0 {
        return Err(CheckPrReviewError::ReviewNotFound(review_num));
    }

    let sorted_reviews = pr_data.sorted_reviews();

    let review = sorted_reviews
        .get(review_num - 1)
        .ok_or(CheckPrReviewError::ReviewNotFound(review_num))?;

    Ok(format_review_with_threads(review, pr_data, options))
}

fn format_review_with_threads(
    review: &Review,
    pr_data: &PrData,
    options: &FormatOptions,
) -> String {
    let mut output = format_review(review, options);

    let review_threads = pr_data.threads_for_review(review.database_id);
    for thread in review_threads {
        output.push_str(&format_thread(thread, options));
    }

    output
}

fn format_review(review: &Review, options: &FormatOptions) -> String {
    let mut output = String::new();
    let formatted_date = format_datetime(&review.created_at);
    output.push_str(&format!(
        "{BG_GRAY} @{} ({formatted_date}) {RESET} {}\n",
        author_login(review),
        state_indicator(review.state)
    ));

    let body = process_body(&review.body, options);
    if !body.is_empty() {
        output.push_str(&body);
        output.push('\n');
    }
    output.push('\n');

    output
}

fn format_thread(thread: &ReviewThread, options: &FormatOptions) -> String {
    let mut output = String::new();
    if let Some(root) = thread.root_comment() {
        output.push_str(&format_comment(
            root,
            "",
            false,
            thread.is_resolved,
            options,
        ));

        for reply in thread.replies() {
            output.push_str(&format_comment(
                reply,
                "  ",
                true,
                thread.is_resolved,
                options,
            ));
        }
    }
    output
}

fn format_comment(
    comment: &Comment,
    indent: &str,
    is_reply: bool,
    is_resolved: bool,
    options: &FormatOptions,
) -> String {
    let mut output = String::new();
    let formatted_date = format_datetime(&comment.created_at);

    if is_reply {
        output.push_str(&format!(
            "{indent}└─ {BG_GRAY} @{} ({formatted_date}) {RESET}\n",
            comment.author_login()
        ));
    } else {
        let resolved_indicator = if is_resolved {
            format!(" {DIM}[resolved]{RESET}")
        } else {
            String::new()
        };
        output.push_str(&format!(
            "{BG_GRAY} @{} ({formatted_date}) {RESET}{resolved_indicator}\n",
            comment.author_login()
        ));
    }

    if !is_reply && let Some(diff_hunk) = &comment.diff_hunk {
        let path = comment.path.as_deref().unwrap_or("unknown");
        output.push_str(&format_diff_with_delta(path, diff_hunk, options.skip_delta));
    }

    let body = process_body(&comment.body, options);
    if is_reply {
        for line in body.lines() {
            output.push_str(&format!("{indent}   {line}\n"));
        }
    } else {
        output.push_str(&body);
        output.push('\n');
    }
    output.push('\n');

    output
}
