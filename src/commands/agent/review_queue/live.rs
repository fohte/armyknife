use anyhow::{Context, Result};

use super::{CRIT_KEY, QueueRuntime, QueueState, Review};
use crate::commands::agent::crit::toggle::{LiveCritPaneRuntime, show_review_pane};
use crate::commands::agent::store;
use crate::commands::agent::types::{Session, resolve_session_option};
use crate::commands::agent::{display_label, repo_name};
use crate::infra::tmux;

/// Registered editor reviews of an agent pane, as JSON.
const EDITORS_OPTION: &str = "@armyknife-review-editors";
const SHOWN_OPTION: &str = "@armyknife-review-shown";
const LAST_OPTION: &str = "@armyknife-review-last";
/// Queue text on the floating pane, with the displayed review in brackets.
const QUEUE_OPTION: &str = "@armyknife-review-queue";
/// Number of waiting reviews on the agent pane while none is displayed.
const COUNT_OPTION: &str = "@armyknife-review-count";

struct CritReview {
    url: String,
    title: String,
}

pub(crate) struct LiveQueueRuntime {
    crit: Option<CritReview>,
}

impl LiveQueueRuntime {
    pub(crate) fn without_crit() -> Self {
        Self { crit: None }
    }

    pub(crate) fn for_session(session: &Session) -> Self {
        let crit = session.crit_urls.last().map(|url| CritReview {
            url: url.clone(),
            title: format!(
                " crit · {} · {} ",
                display_label(Some(session), &session.session_id),
                repo_name(&session.cwd)
            ),
        });
        Self { crit }
    }

    /// `None` when `pane_id` is not an agent pane, whose reviews are not queued.
    pub(crate) fn for_pane(pane_id: &str) -> Option<Self> {
        let session_id = resolve_session_option(|option| tmux::get_pane_option(pane_id, option))?;
        match store::load_session(&session_id) {
            Ok(Some(session)) => Some(Self::for_session(&session)),
            Ok(None) => Some(Self::without_crit()),
            Err(error) => {
                tracing::warn!(
                    event = "agent.review_queue.session_load_failed",
                    session = %session_id,
                    error = %error,
                );
                Some(Self::without_crit())
            }
        }
    }

    fn displayed_panes(parent: &str) -> Result<Vec<String>> {
        let mut panes = tmux::find_crit_panes_for_parent(parent)?;
        panes.extend(tmux::find_editor_panes_for_parent(parent)?);
        Ok(panes)
    }
}

fn unset_option(pane_id: &str, option: &str) -> Result<()> {
    tmux::run_tmux(&["set-option", "-p", "-u", "-t", pane_id, option])?;
    Ok(())
}

fn set_or_unset(pane_id: &str, option: &str, value: Option<&str>) -> Result<()> {
    match value {
        Some(value) => tmux::set_pane_option(pane_id, option, value)?,
        None => unset_option(pane_id, option)?,
    }
    Ok(())
}

impl QueueRuntime for LiveQueueRuntime {
    fn editors(&self, parent: &str) -> Result<Vec<Review>> {
        let Some(json) = tmux::get_pane_option(parent, EDITORS_OPTION) else {
            return Ok(Vec::new());
        };
        serde_json::from_str(&json).context("failed to parse the registered editor reviews")
    }

    fn set_editors(&self, parent: &str, editors: &[Review]) -> Result<()> {
        let json = (!editors.is_empty())
            .then(|| serde_json::to_string(editors))
            .transpose()
            .context("failed to serialize the registered editor reviews")?;
        set_or_unset(parent, EDITORS_OPTION, json.as_deref())
    }

    fn editor_alive(&self, session: &str) -> Result<bool> {
        crate::infra::shpool::session_is_alive(session)
    }

    fn has_crit(&self) -> bool {
        self.crit.is_some()
    }

    fn state(&self, parent: &str) -> QueueState {
        QueueState {
            shown: tmux::get_pane_option(parent, SHOWN_OPTION),
            last: tmux::get_pane_option(parent, LAST_OPTION),
        }
    }

    fn set_state(&self, parent: &str, state: &QueueState) -> Result<()> {
        set_or_unset(parent, SHOWN_OPTION, state.shown.as_deref())?;
        set_or_unset(parent, LAST_OPTION, state.last.as_deref())
    }

    fn is_displayed(&self, parent: &str) -> Result<bool> {
        Ok(!Self::displayed_panes(parent)?.is_empty())
    }

    fn show(&self, parent: &str, review: &Review) -> Result<()> {
        if review.key == CRIT_KEY {
            for pane in tmux::find_editor_panes_for_parent(parent)? {
                tmux::kill_pane(&pane)?;
            }
            let crit = self
                .crit
                .as_ref()
                .context("No crit review is associated with this agent session")?;
            return show_review_pane(parent, Some(&crit.url), &crit.title, &LiveCritPaneRuntime);
        }

        self.hide(parent)?;
        tmux::focus_pane(parent)?;
        tmux::open_editor_pane(tmux::EditorPaneSpec {
            parent_pane_id: parent,
            title: &review.label,
            shpool_session: &review.key,
            command: None,
            force_shpool_attach: true,
        })?;
        Ok(())
    }

    fn hide(&self, parent: &str) -> Result<()> {
        for pane in tmux::find_crit_panes_for_parent(parent)? {
            tmux::close_crit_pane(&pane)?;
        }
        for pane in tmux::find_editor_panes_for_parent(parent)? {
            // Killing the pane only detaches the shpool session.
            tmux::kill_pane(&pane)?;
        }
        Ok(())
    }

    fn set_labels(&self, parent: &str, queue_text: Option<&str>, count: usize) -> Result<()> {
        match queue_text {
            Some(text) => {
                for pane in Self::displayed_panes(parent)? {
                    tmux::set_pane_option(&pane, QUEUE_OPTION, text)?;
                }
                unset_option(parent, COUNT_OPTION)?;
            }
            None => {
                let count = (count > 0).then(|| count.to_string());
                set_or_unset(parent, COUNT_OPTION, count.as_deref())?;
            }
        }
        // The pane border format is evaluated on layout changes only.
        tmux::run_tmux(&["set-hook", "-R", "-t", parent, "window-layout-changed"])?;
        Ok(())
    }
}
