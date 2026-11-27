use crate::commands::agent::store::WatchWebActionEventKind;

pub(in crate::commands::agent::watch_web) const VIBE_MODE_ON_TAG: &str = "armyknife-vibe-mode-on";
pub(in crate::commands::agent::watch_web) const VIBE_MODE_OFF_TAG: &str = "armyknife-vibe-mode-off";
pub(in crate::commands::agent::watch_web) const TODO_COMPLETED_TAG: &str =
    "armyknife-todo-completed";
pub(in crate::commands::agent::watch_web) const TODO_REJECTED_TAG: &str = "armyknife-todo-rejected";
pub(in crate::commands::agent::watch_web) const DECISION_CHANGE_TAG: &str =
    "armyknife-decision-change";

const ACTION_MESSAGE_ID_PREFIX: &str = "armyknife-action:";

pub(in crate::commands::agent::watch_web) const VIBE_MODE_ON_NOTICE: &str =
    "watch-web からユーザーが vibe mode を on にしました。";
pub(in crate::commands::agent::watch_web) const VIBE_MODE_OFF_NOTICE: &str = "watch-web からユーザーが vibe mode を off にしました。祖先セッションから引き継いでいる場合、vibe mode は引き続き有効です。";

pub(in crate::commands::agent::watch_web) fn new_action_message_id() -> String {
    format!("{ACTION_MESSAGE_ID_PREFIX}{}", uuid::Uuid::new_v4())
}

pub(super) fn is_action_event_tag(tag: &str) -> bool {
    matches!(
        tag,
        TODO_COMPLETED_TAG
            | TODO_REJECTED_TAG
            | DECISION_CHANGE_TAG
            | VIBE_MODE_ON_TAG
            | VIBE_MODE_OFF_TAG
    )
}

pub(in crate::commands::agent::watch_web) fn event_text_for_kind(
    kind: WatchWebActionEventKind,
    body: &str,
) -> String {
    match kind {
        WatchWebActionEventKind::VibeModeOn => "Vibe mode turned on".to_owned(),
        WatchWebActionEventKind::VibeModeOff => "Vibe mode turned off".to_owned(),
        WatchWebActionEventKind::TodoCompleted
        | WatchWebActionEventKind::TodoRejected
        | WatchWebActionEventKind::DecisionChange => one_line(body),
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
