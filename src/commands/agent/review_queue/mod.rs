//! Orders the reviews an agent pane is waiting on (editor reviews in start
//! order, then Crit) and shows one of them at a time in a floating pane.

mod live;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub(crate) use live::LiveQueueRuntime;

pub(crate) const CRIT_KEY: &str = "crit";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Review {
    /// shpool session name for an editor review, `CRIT_KEY` for Crit.
    pub(crate) key: String,
    pub(crate) label: String,
}

impl Review {
    pub(crate) fn editor(session: &str, title: &str) -> Self {
        Self {
            key: session.to_string(),
            label: title.to_string(),
        }
    }

    fn crit() -> Self {
        Self {
            key: CRIT_KEY.to_string(),
            label: CRIT_KEY.to_string(),
        }
    }
}

/// `shown` is the review meant to be on screen, which stays set after its
/// pane is closed by other means; `last` is the one to bring back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct QueueState {
    pub(crate) shown: Option<String>,
    pub(crate) last: Option<String>,
}

pub(crate) trait QueueRuntime {
    /// Registered editor reviews in start order, including finished ones that
    /// have not been unregistered.
    fn editors(&self, parent: &str) -> Result<Vec<Review>>;
    fn set_editors(&self, parent: &str, editors: &[Review]) -> Result<()>;
    fn editor_alive(&self, session: &str) -> Result<bool>;
    fn has_crit(&self) -> bool;
    fn state(&self, parent: &str) -> QueueState;
    fn set_state(&self, parent: &str, state: &QueueState) -> Result<()>;
    /// Whether a review pane of `parent` exists.
    fn is_displayed(&self, parent: &str) -> Result<bool>;
    /// Replaces the displayed review pane with `review`.
    fn show(&self, parent: &str, review: &Review) -> Result<()>;
    /// Closes the displayed review pane. The review itself keeps running.
    fn hide(&self, parent: &str) -> Result<()>;
    /// `queue_text` is set on the displayed pane, otherwise `count` on `parent`.
    fn set_labels(&self, parent: &str, queue_text: Option<&str>, count: usize) -> Result<()>;
}

/// Where a newly started editor review goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placement {
    Visible,
    Hidden,
}

fn ordered(
    runtime: &impl QueueRuntime,
    parent: &str,
    finished: Option<&str>,
) -> Result<Vec<Review>> {
    let mut queue = Vec::new();
    for editor in runtime.editors(parent)? {
        if finished == Some(editor.key.as_str()) || runtime.editor_alive(&editor.key)? {
            queue.push(editor);
        }
    }
    if runtime.has_crit() || finished == Some(CRIT_KEY) {
        queue.push(Review::crit());
    }
    Ok(queue)
}

fn next_after<'a>(queue: &'a [Review], key: &str) -> Option<&'a Review> {
    let next = queue
        .iter()
        .position(|review| review.key == key)
        .map_or(0, |index| (index + 1) % queue.len());
    queue.get(next)
}

fn render(queue: &[Review], shown: Option<&str>) -> String {
    queue
        .iter()
        .map(|review| {
            if shown == Some(review.key.as_str()) {
                format!("[{}]", review.label)
            } else {
                review.label.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn displayed_key(runtime: &impl QueueRuntime, parent: &str) -> Result<Option<String>> {
    let shown = runtime.state(parent).shown;
    if shown.is_some() && runtime.is_displayed(parent)? {
        return Ok(shown);
    }
    Ok(None)
}

pub(crate) fn refresh(runtime: &impl QueueRuntime, parent: &str) -> Result<()> {
    let queue = ordered(runtime, parent, None)?;
    let shown = displayed_key(runtime, parent)?;
    let text = shown.as_deref().map(|key| render(&queue, Some(key)));
    runtime.set_labels(parent, text.as_deref(), queue.len())
}

fn show(runtime: &impl QueueRuntime, parent: &str, review: &Review) -> Result<()> {
    runtime.show(parent, review)?;
    runtime.set_state(
        parent,
        &QueueState {
            shown: Some(review.key.clone()),
            last: Some(review.key.clone()),
        },
    )?;
    refresh(runtime, parent)
}

/// alt-r: brings back the last review, or switches to the next one.
pub(crate) fn cycle(runtime: &impl QueueRuntime, parent: &str) -> Result<()> {
    let queue = ordered(runtime, parent, None)?;
    ensure!(
        !queue.is_empty(),
        "No review is waiting for this agent pane"
    );
    let state = runtime.state(parent);
    let target = match displayed_key(runtime, parent)? {
        Some(shown) => {
            let Some(next) = next_after(&queue, &shown).filter(|next| next.key != shown) else {
                return refresh(runtime, parent);
            };
            next
        }
        None => state
            .last
            .as_deref()
            .and_then(|last| queue.iter().find(|review| review.key == last))
            .unwrap_or(&queue[0]),
    };
    show(runtime, parent, target)
}

/// Brings Crit to the front, e.g. from a notification.
pub(crate) fn show_crit(runtime: &impl QueueRuntime, parent: &str) -> Result<()> {
    ensure!(
        runtime.has_crit(),
        "No crit review is associated with this agent session"
    );
    show(runtime, parent, &Review::crit())
}

/// alt-shift-r: closes the displayed review without ending it.
pub(crate) fn hide(runtime: &impl QueueRuntime, parent: &str) -> Result<()> {
    let state = runtime.state(parent);
    let displayed = runtime.is_displayed(parent)?;
    if !displayed && state.shown.is_none() {
        return Ok(());
    }
    if displayed {
        runtime.hide(parent)?;
    }
    runtime.set_state(
        parent,
        &QueueState {
            shown: None,
            last: state.shown.or(state.last),
        },
    )?;
    refresh(runtime, parent)
}

/// Registers an editor review. The caller creates the session and calls
/// `refresh` afterwards, because the session is not alive yet.
pub(crate) fn editor_started(
    runtime: &impl QueueRuntime,
    parent: &str,
    review: Review,
) -> Result<Placement> {
    let displayed = displayed_key(runtime, parent)?;
    let mut editors = runtime.editors(parent)?;
    let key = review.key.clone();
    editors.push(review);
    runtime.set_editors(parent, &editors)?;

    if displayed.as_deref().is_some_and(|shown| shown != CRIT_KEY) {
        return Ok(Placement::Hidden);
    }
    if displayed.is_some() {
        runtime.hide(parent)?;
    }
    runtime.set_state(
        parent,
        &QueueState {
            shown: Some(key.clone()),
            last: Some(key),
        },
    )?;
    Ok(Placement::Visible)
}

/// Removes a finished review and, if it was on screen, shows the next one.
pub(crate) fn review_finished(runtime: &impl QueueRuntime, parent: &str, key: &str) -> Result<()> {
    let state = runtime.state(parent);
    let before = ordered(runtime, parent, Some(key))?;
    if key != CRIT_KEY {
        let remaining = runtime
            .editors(parent)?
            .into_iter()
            .filter(|editor| editor.key != key)
            .collect::<Vec<_>>();
        runtime.set_editors(parent, &remaining)?;
    }
    if state.shown.as_deref() != Some(key) {
        return refresh(runtime, parent);
    }

    if runtime.is_displayed(parent)? {
        runtime.hide(parent)?;
    }
    let after = before
        .iter()
        .filter(|review| review.key != key)
        .cloned()
        .collect::<Vec<_>>();
    let position = before.iter().position(|review| review.key == key);
    let next = match (position, after.len()) {
        (_, 0) => None,
        (Some(index), len) => after.get(index % len),
        (None, _) => after.first(),
    };
    match next {
        Some(next) => show(runtime, parent, next),
        None => {
            runtime.set_state(parent, &QueueState::default())?;
            refresh(runtime, parent)
        }
    }
}

#[cfg(test)]
mod tests;
