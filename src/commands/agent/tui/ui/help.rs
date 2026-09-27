use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::Paragraph,
};

use crate::commands::agent::tui::app::{App, AppMode, NarrowScreen, View};

pub(super) fn render_help_lines(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    let help = Paragraph::new(Text::from(lines)).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, area);
}

pub(super) fn build_help_lines(app: &App) -> Vec<Line<'static>> {
    let bold = Style::default().add_modifier(Modifier::BOLD);

    if app.view == View::Clean {
        return build_clean_help_lines(app);
    }

    if let Some(line) = clean_status_line(app, bold) {
        return vec![line];
    }

    build_session_help_lines(app, bold)
}

fn clean_status_line(app: &App, bold: Style) -> Option<Line<'static>> {
    let progress_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let progress = app.clean_progress.as_ref()?;
    Some(Line::from(vec![
        Span::raw("  "),
        Span::styled(progress.render_line(), progress_style),
        Span::raw("   "),
        Span::styled("q", bold),
        Span::raw(": quit"),
    ]))
}

fn build_compact_help_line(bold: Style, hints: &[(&str, &str)]) -> Vec<Line<'static>> {
    let mut spans = vec![
        Span::raw(" "),
        Span::styled("?", bold),
        Span::raw(": keys   "),
    ];
    for (i, (key, label)) in hints.iter().enumerate() {
        spans.push(Span::styled((*key).to_string(), bold));
        let sep = if i + 1 == hints.len() { "" } else { "   " };
        spans.push(Span::raw(format!(": {label}{sep}")));
    }
    vec![Line::from(spans)]
}

fn build_session_help_lines(app: &App, bold: Style) -> Vec<Line<'static>> {
    if app.mode == AppMode::Normal && app.narrow_layout {
        return build_narrow_session_help_lines(app, bold);
    }

    if app.mode == AppMode::Normal && app.sidebar_focused {
        if app.show_help {
            return vec![
                Line::from(vec![
                    Span::styled("  j/k", bold),
                    Span::raw(": move  "),
                    Span::styled("Enter", bold),
                    Span::raw(": filter  "),
                    Span::styled("h/←", bold),
                    Span::raw(": parent / collapse"),
                ]),
                Line::from(vec![
                    Span::styled("  l/→", bold),
                    Span::raw(": expand  "),
                    Span::styled("r", bold),
                    Span::raw(": refresh  "),
                    Span::styled("Tab", bold),
                    Span::raw(": list  "),
                    Span::styled("C-b", bold),
                    Span::raw(": hide  "),
                    Span::styled("q", bold),
                    Span::raw(": quit"),
                ]),
            ];
        }
        return build_compact_help_line(
            bold,
            &[
                ("j/k", "move"),
                ("Enter", "filter"),
                ("r", "refresh"),
                ("Tab", "list"),
                ("C-b", "hide"),
            ],
        );
    }

    match &app.mode {
        AppMode::Confirm {
            is_alive,
            worktree_cleanup,
            ..
        } => {
            let base = if *is_alive {
                "Stop and delete session"
            } else {
                "Delete session"
            };
            let suffix = if worktree_cleanup.is_some() {
                " (last in worktree; also deletes worktree, branch, tmux windows)"
            } else {
                ""
            };
            let prompt = format!("{base}{suffix}?");
            let warn_style = Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD);
            vec![Line::from(vec![
                Span::styled(format!("  {prompt} "), warn_style),
                Span::styled("y", bold),
                Span::raw(": yes  "),
                Span::styled("n/Esc", bold),
                Span::raw(": cancel"),
            ])]
        }
        AppMode::Search => vec![Line::from(vec![
            Span::styled("  C-n/C-p", bold),
            Span::raw(": move  "),
            Span::styled("Enter", bold),
            Span::raw(": focus  "),
            Span::styled("Esc", bold),
            Span::raw(": cancel"),
        ])],
        AppMode::Edit { .. } => vec![Line::from(vec![
            Span::styled("  Enter", bold),
            Span::raw(": save  "),
            Span::styled("Ctrl+g", bold),
            Span::raw(": generate  "),
            Span::styled("Esc", bold),
            Span::raw(": cancel"),
        ])],
        AppMode::Normal if app.show_help && app.has_filter() => {
            let mut lines = vec![
                Line::from(vec![
                    Span::styled("  j/k", bold),
                    Span::raw(": move  "),
                    Span::styled("f", bold),
                    Span::raw(": focus  "),
                    Span::styled("r", bold),
                    Span::raw(": resume  "),
                    Span::styled("o", bold),
                    Span::raw(": open crit  "),
                    Span::styled("d", bold),
                    Span::raw(": delete  "),
                    Span::styled("/", bold),
                    Span::raw(": edit  "),
                    Span::styled("q", bold),
                    Span::raw(": quit"),
                ]),
                Line::from(vec![
                    Span::styled("  h/←", bold),
                    Span::raw(": parent  "),
                    Span::styled("→/l", bold),
                    Span::raw(": drill down  "),
                    Span::styled("C-r/w/s/p", bold),
                    Span::raw(": filter  "),
                    Span::styled("Esc", bold),
                    Span::raw(": clear"),
                ]),
            ];
            if app.sidebar_available {
                lines.push(Line::from(vec![
                    Span::styled("  Tab", bold),
                    Span::raw(": focus  "),
                    Span::styled("C-b", bold),
                    Span::raw(": sidebar"),
                ]));
            }
            lines
        }
        AppMode::Normal if app.show_help => {
            let mut lines = vec![
                Line::from(vec![
                    Span::styled("  j/k", bold),
                    Span::raw(": move  "),
                    Span::styled("f", bold),
                    Span::raw(": focus  "),
                    Span::styled("r", bold),
                    Span::raw(": resume  "),
                    Span::styled("p", bold),
                    Span::raw(": preview  "),
                    Span::styled("t", bold),
                    Span::raw(": open task  "),
                    Span::styled("d", bold),
                    Span::raw(": delete"),
                ]),
                Line::from(vec![
                    Span::styled("  1-9", bold),
                    Span::raw(": quick  "),
                    Span::styled("/", bold),
                    Span::raw(": search  "),
                    Span::styled("h/←", bold),
                    Span::raw(": parent  "),
                    Span::styled("→/l", bold),
                    Span::raw(": drill down"),
                ]),
                Line::from(vec![
                    Span::styled("  C-r/w/s/p", bold),
                    Span::raw(": filter  "),
                    Span::styled("o", bold),
                    Span::raw(": open crit  "),
                    Span::styled("q", bold),
                    Span::raw(": quit"),
                ]),
            ];
            if app.sidebar_available {
                lines.push(Line::from(vec![
                    Span::styled("  Tab", bold),
                    Span::raw(": focus  "),
                    Span::styled("C-b", bold),
                    Span::raw(": toggle sidebar"),
                ]));
            }
            lines
        }
        AppMode::Normal if app.has_filter() && app.sidebar_available => build_compact_help_line(
            bold,
            &[
                ("/", "search"),
                ("Tab", "focus"),
                ("C-b", "sidebar"),
                ("Esc", "clear filter"),
                ("q", "quit"),
            ],
        ),
        AppMode::Normal if app.has_filter() => build_compact_help_line(
            bold,
            &[("/", "search"), ("Esc", "clear filter"), ("q", "quit")],
        ),
        AppMode::Normal if app.sidebar_available => build_compact_help_line(
            bold,
            &[
                ("/", "search"),
                ("Tab", "focus"),
                ("C-b", "sidebar"),
                ("q", "quit"),
            ],
        ),
        AppMode::Normal => build_compact_help_line(bold, &[("/", "search"), ("q", "quit")]),
    }
}

fn build_narrow_session_help_lines(app: &App, bold: Style) -> Vec<Line<'static>> {
    match app.narrow_screen {
        NarrowScreen::Sidebar if app.show_help => vec![
            Line::from(vec![
                Span::styled("  j/k", bold),
                Span::raw(": move  "),
                Span::styled("Enter", bold),
                Span::raw(": select  "),
                Span::styled("h/←", bold),
                Span::raw(": parent / collapse"),
            ]),
            Line::from(vec![
                Span::styled("  l/→", bold),
                Span::raw(": expand  "),
                Span::styled("r", bold),
                Span::raw(": refresh  "),
                Span::styled("Esc", bold),
                Span::raw(": clear filter  "),
                Span::styled("q", bold),
                Span::raw(": quit"),
            ]),
        ],
        NarrowScreen::Sidebar => build_compact_help_line(
            bold,
            &[
                ("j/k", "move"),
                ("h/l", "tree"),
                ("Enter", "select"),
                ("r", "refresh"),
            ],
        ),
        NarrowScreen::SessionList if app.show_help => {
            let escape_label = if app.has_non_sidebar_filter() {
                "clear filters"
            } else {
                "tasks"
            };
            vec![
                Line::from(vec![
                    Span::styled("  j/k", bold),
                    Span::raw(": move  "),
                    Span::styled("f", bold),
                    Span::raw(": focus  "),
                    Span::styled("r", bold),
                    Span::raw(": resume  "),
                    Span::styled("p", bold),
                    Span::raw(": preview  "),
                    Span::styled("t", bold),
                    Span::raw(": open task"),
                ]),
                Line::from(vec![
                    Span::styled("  d", bold),
                    Span::raw(": delete  "),
                    Span::styled("1-9", bold),
                    Span::raw(": quick select  "),
                    Span::styled("/", bold),
                    Span::raw(": search  "),
                    Span::styled("h/←", bold),
                    Span::raw(": parent  "),
                    Span::styled("→/l", bold),
                    Span::raw(": drill down"),
                ]),
                Line::from(vec![
                    Span::styled("  C-r/w/s/p", bold),
                    Span::raw(": filter  "),
                    Span::styled("o", bold),
                    Span::raw(": open crit  "),
                    Span::styled("Esc", bold),
                    Span::raw(format!(": {escape_label}  ")),
                    Span::styled("q", bold),
                    Span::raw(": quit"),
                ]),
            ]
        }
        NarrowScreen::SessionList => {
            let escape_label = if app.has_non_sidebar_filter() {
                "clear filter"
            } else {
                "tasks"
            };
            build_compact_help_line(
                bold,
                &[("/", "search"), ("Esc", escape_label), ("q", "quit")],
            )
        }
    }
}

fn build_clean_help_lines(app: &App) -> Vec<Line<'static>> {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::DarkGray);
    let warn = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    let (to_delete, kept_active) = app.clean_view.summary();
    let prompt = if to_delete == 0 {
        "  Nothing to clean. ".to_string()
    } else if kept_active > 0 {
        format!(
            "  Clean {to_delete} worktree{} ({kept_active} active excluded)? ",
            if to_delete == 1 { "" } else { "s" }
        )
    } else {
        format!(
            "  Clean {to_delete} worktree{}? ",
            if to_delete == 1 { "" } else { "s" }
        )
    };

    let help_line = Line::from(vec![
        Span::styled("  j/k", bold),
        Span::raw(": move  "),
        Span::styled("Enter", bold),
        Span::raw(": toggle / focus session  "),
        Span::styled("y", bold),
        Span::raw(": run  "),
        Span::styled("n/Esc/q", bold),
        Span::raw(": cancel"),
    ]);
    let prompt_line = if to_delete == 0 {
        Line::from(vec![
            Span::styled(prompt, dim),
            Span::styled("n/Esc/q", bold),
            Span::raw(": back"),
        ])
    } else {
        Line::from(vec![
            Span::styled(prompt, warn),
            Span::styled("y", bold),
            Span::raw(": run  "),
            Span::styled("N", bold),
            Span::raw(": cancel"),
        ])
    };
    vec![help_line, prompt_line]
}
