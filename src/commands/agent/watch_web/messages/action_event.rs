pub(in crate::commands::agent::watch_web) const VIBE_MODE_ON_TAG: &str = "armyknife-vibe-mode-on";
pub(in crate::commands::agent::watch_web) const VIBE_MODE_OFF_TAG: &str = "armyknife-vibe-mode-off";

pub(in crate::commands::agent::watch_web) const VIBE_MODE_ON_NOTICE: &str =
    "watch-web からユーザーが vibe mode を on にしました。";
pub(in crate::commands::agent::watch_web) const VIBE_MODE_OFF_NOTICE: &str = "watch-web からユーザーが vibe mode を off にしました。祖先セッションから引き継いでいる場合、vibe mode は引き続き有効です。";

pub(super) fn is_vibe_mode_tag(tag: &str) -> bool {
    matches!(tag, VIBE_MODE_ON_TAG | VIBE_MODE_OFF_TAG)
}

pub(super) fn event_text(tag: &str, body: &str) -> Option<String> {
    match tag {
        "armyknife-todo-completed" | "armyknife-todo-rejected" | "armyknife-decision-change" => {
            Some(one_line(body))
        }
        VIBE_MODE_ON_TAG if body == VIBE_MODE_ON_NOTICE => Some("Vibe mode turned on".to_owned()),
        VIBE_MODE_OFF_TAG if body == VIBE_MODE_OFF_NOTICE => {
            Some("Vibe mode turned off".to_owned())
        }
        _ => None,
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
