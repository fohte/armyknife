use chrono::{DateTime, Utc};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::ListItem,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::commands::agent::tui::app::App;
use crate::commands::agent::tui::progress::{ProgressMark, ProgressRow};
use crate::commands::agent::tui::session_rows::inline_progress::InlineProgressEntry;

use super::super::helpers::{DIM_FG, REPO_COLUMN_WIDTH};
use super::items::{
    TASK_NUMBER_COLUMN_WIDTH, build_session_item_with_annotation, tree_branch_text,
    work_type_column_width,
};

pub(super) fn build_inline_progress_item(
    entry: &InlineProgressEntry<'_>,
    app: &App,
    now: DateTime<Utc>,
    term_width: usize,
) -> ListItem<'static> {
    if let (Some(session), Some(ProgressRow::ChecklistPullRequest { row, .. })) =
        (&entry.session, entry.progress.as_ref())
    {
        return build_session_item_with_annotation(
            session,
            app,
            now,
            term_width,
            "",
            Some(&format!(
                "PR {} {}",
                row.number
                    .map_or_else(|| "#?".to_string(), |number| format!("#{number}")),
                stage_marks(
                    row.stages.implementation,
                    row.stages.diff_review,
                    row.stages.pr_body,
                    row.stages.ci,
                    row.stages.merge
                ),
            )),
        );
    }

    let branch = tree_branch_text(Some(&entry.tree_prefix), entry.is_last_sibling);
    let branch_width = branch.width();
    let work_type_width = work_type_column_width(app);
    let title_width = term_width.saturating_sub(
        2 + REPO_COLUMN_WIDTH + TASK_NUMBER_COLUMN_WIDTH + work_type_width + branch_width,
    );
    let (status, title, style) = match (entry.progress.as_ref(), entry.label.as_deref()) {
        (_, Some(label)) => ("!", label.to_string(), Style::default().fg(Color::Yellow)),
        (
            Some(ProgressRow::ImplementationHeader {
                merged_pr_count,
                total_pr_count,
                collapsed,
            }),
            _,
        ) => (
            if *collapsed { "▸" } else { "▾" },
            format!("Implementation plan · {merged_pr_count}/{total_pr_count} PRs merged"),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        (
            Some(ProgressRow::BatchHeader {
                title,
                merged_pr_count,
                collapsed,
                ..
            }),
            _,
        ) => (
            if *collapsed { "▸" } else { "▾" },
            format!("Batch: {title} · {merged_pr_count} PRs merged"),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        (Some(ProgressRow::PlannedPullRequest { title, .. }), _) => (
            "○",
            format!("Planned PR · {title}"),
            Style::default().fg(DIM_FG),
        ),
        (Some(ProgressRow::ChecklistPullRequest { row, .. }), _) => (
            stage_status(row.stages.merge, row.stages.ci),
            format!(
                "PR {} · {} · {}",
                row.number
                    .map_or_else(|| "#?".to_string(), |number| format!("#{number}")),
                row.title,
                stage_marks(
                    row.stages.implementation,
                    row.stages.diff_review,
                    row.stages.pr_body,
                    row.stages.ci,
                    row.stages.merge
                ),
            ),
            Style::default(),
        ),
        (Some(ProgressRow::PullRequest(row)), _) => (
            stage_status(row.stages.merge, row.stages.ci),
            format!(
                "PR {} · {}",
                row.number
                    .map_or_else(|| "#?".to_string(), |number| format!("#{number}")),
                row.title
            ),
            Style::default(),
        ),
        (Some(ProgressRow::Work(row)), _) => (
            row.mark.symbol(),
            row.status_label
                .clone()
                .unwrap_or_else(|| "Session".to_string()),
            Style::default().fg(DIM_FG),
        ),
        (Some(ProgressRow::UnmatchedDelegatesHeader { collapsed }), _) => (
            if *collapsed { "▸" } else { "▾" },
            "Unmatched delegates".to_string(),
            Style::default().fg(DIM_FG),
        ),
        (None, _) => ("·", String::new(), Style::default().fg(DIM_FG)),
    };
    let title = truncate_inline_title(&title, title_width);
    let spans = vec![
        Span::styled(
            status.to_string(),
            Style::default().fg(status_color(status)),
        ),
        Span::raw(" "),
        Span::raw(" ".repeat(REPO_COLUMN_WIDTH)),
        Span::raw(" ".repeat(TASK_NUMBER_COLUMN_WIDTH)),
        Span::styled(branch, Style::default().fg(DIM_FG)),
        Span::raw(" ".repeat(work_type_width)),
        Span::styled(title, style),
    ];
    ListItem::new(Line::from(spans))
}

fn stage_marks(
    implementation: ProgressMark,
    review: ProgressMark,
    pr_body: ProgressMark,
    ci: ProgressMark,
    merge: ProgressMark,
) -> String {
    format!(
        "I:{} R:{} B:{} CI:{} M:{}",
        implementation.symbol(),
        review.symbol(),
        pr_body.symbol(),
        ci.symbol(),
        merge.symbol(),
    )
}

fn stage_status(merge: ProgressMark, ci: ProgressMark) -> &'static str {
    if merge == ProgressMark::Done {
        "✓"
    } else if ci == ProgressMark::Waiting {
        "◷"
    } else {
        merge.symbol()
    }
}

fn status_color(status: &str) -> Color {
    match status {
        "✓" => Color::Green,
        "◷" => Color::Yellow,
        "!" => Color::Red,
        "▸" | "▾" => Color::Cyan,
        _ => DIM_FG,
    }
}

fn truncate_inline_title(title: &str, width: usize) -> String {
    if title.width() <= width {
        title.to_string()
    } else {
        let mut output = String::new();
        let mut used_width = 0;
        for ch in title.chars() {
            let char_width = ch.width().unwrap_or(0);
            if used_width + char_width > width.saturating_sub(1) {
                break;
            }
            output.push(ch);
            used_width += char_width;
        }
        if width > 0 {
            output.push('…');
        }
        output
    }
}
