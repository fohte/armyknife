use std::cell::RefCell;

use rstest::rstest;

use super::*;

const PARENT: &str = "%agent";

#[derive(Default)]
struct Fake {
    editors: RefCell<Vec<Review>>,
    alive: Vec<&'static str>,
    crit: bool,
    state: RefCell<QueueState>,
    displayed: RefCell<bool>,
    log: RefCell<Vec<String>>,
}

impl Fake {
    /// `editors` are alive; `shown` is displayed.
    fn new(editors: &[&'static str], crit: bool, shown: Option<&str>, last: Option<&str>) -> Self {
        Self {
            editors: RefCell::new(editors.iter().map(|key| Review::editor(key, key)).collect()),
            alive: editors.to_vec(),
            crit,
            state: RefCell::new(QueueState {
                shown: shown.map(str::to_string),
                last: last.map(str::to_string),
            }),
            displayed: RefCell::new(shown.is_some()),
            ..Self::default()
        }
    }

    /// The pane of the review meant to be shown was closed by other means.
    fn closed(self) -> Self {
        *self.displayed.borrow_mut() = false;
        self
    }

    fn log(&self, entry: String) {
        self.log.borrow_mut().push(entry);
    }

    fn outcome(&self) -> Outcome {
        Outcome {
            log: self.log.borrow().clone(),
            editors: self
                .editors
                .borrow()
                .iter()
                .map(|editor| editor.key.clone())
                .collect(),
            state: self.state.borrow().clone(),
            displayed: *self.displayed.borrow(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    log: Vec<String>,
    editors: Vec<String>,
    state: QueueState,
    displayed: bool,
}

fn outcome(
    log: &[&str],
    editors: &[&str],
    shown: Option<&str>,
    last: Option<&str>,
    displayed: bool,
) -> Outcome {
    Outcome {
        log: log.iter().map(|entry| entry.to_string()).collect(),
        editors: editors.iter().map(|key| key.to_string()).collect(),
        state: QueueState {
            shown: shown.map(str::to_string),
            last: last.map(str::to_string),
        },
        displayed,
    }
}

impl QueueRuntime for Fake {
    fn editors(&self, _: &str) -> Result<Vec<Review>> {
        Ok(self.editors.borrow().clone())
    }

    fn set_editors(&self, _: &str, editors: &[Review]) -> Result<()> {
        *self.editors.borrow_mut() = editors.to_vec();
        Ok(())
    }

    fn editor_alive(&self, session: &str) -> Result<bool> {
        Ok(self.alive.contains(&session))
    }

    fn has_crit(&self) -> bool {
        self.crit
    }

    fn state(&self, _: &str) -> QueueState {
        self.state.borrow().clone()
    }

    fn set_state(&self, _: &str, state: &QueueState) -> Result<()> {
        *self.state.borrow_mut() = state.clone();
        Ok(())
    }

    fn is_displayed(&self, _: &str) -> Result<bool> {
        Ok(*self.displayed.borrow())
    }

    fn show(&self, _: &str, review: &Review) -> Result<()> {
        *self.displayed.borrow_mut() = true;
        self.log(format!("show:{}", review.key));
        Ok(())
    }

    fn hide(&self, _: &str) -> Result<()> {
        *self.displayed.borrow_mut() = false;
        self.log("hide".to_string());
        Ok(())
    }

    fn set_labels(&self, _: &str, queue_text: Option<&str>, count: usize) -> Result<()> {
        self.log(format!("labels:{queue_text:?}:{count}"));
        Ok(())
    }
}

#[rstest]
#[case::shows_the_first_review_when_nothing_was_shown(
    Fake::new(&["e1", "e2"], true, None, None),
    outcome(&["show:e1", "labels:Some(\"[e1] · e2 · crit\"):3"], &["e1", "e2"], Some("e1"), Some("e1"), true),
)]
#[case::brings_back_the_last_review(
    Fake::new(&["e1", "e2"], true, None, Some("e2")),
    outcome(&["show:e2", "labels:Some(\"e1 · [e2] · crit\"):3"], &["e1", "e2"], Some("e2"), Some("e2"), true),
)]
#[case::falls_back_to_the_first_when_the_last_review_ended(
    Fake::new(&["e1"], true, None, Some("gone")),
    outcome(&["show:e1", "labels:Some(\"[e1] · crit\"):2"], &["e1"], Some("e1"), Some("e1"), true),
)]
#[case::switches_to_the_next_review(
    Fake::new(&["e1", "e2"], true, Some("e1"), Some("e1")),
    outcome(&["show:e2", "labels:Some(\"e1 · [e2] · crit\"):3"], &["e1", "e2"], Some("e2"), Some("e2"), true),
)]
#[case::wraps_from_crit_to_the_first_review(
    Fake::new(&["e1", "e2"], true, Some("crit"), Some("crit")),
    outcome(&["show:e1", "labels:Some(\"[e1] · e2 · crit\"):3"], &["e1", "e2"], Some("e1"), Some("e1"), true),
)]
#[case::keeps_a_single_review(
    Fake::new(&[], true, Some("crit"), Some("crit")),
    outcome(&["labels:Some(\"[crit]\"):1"], &[], Some("crit"), Some("crit"), true),
)]
#[case::treats_a_pane_closed_by_other_means_as_hidden(
    Fake::new(&["e1", "e2"], false, Some("e1"), Some("e1")).closed(),
    outcome(&["show:e1", "labels:Some(\"[e1] · e2\"):2"], &["e1", "e2"], Some("e1"), Some("e1"), true),
)]
fn cycle_selects_the_review(#[case] runtime: Fake, #[case] expected: Outcome) {
    cycle(&runtime, PARENT).expect("cycle should succeed");

    assert_eq!(runtime.outcome(), expected);
}

#[test]
fn cycle_fails_without_reviews() {
    let runtime = Fake::new(&[], false, None, None);

    assert_eq!(
        cycle(&runtime, PARENT).map_err(|error| error.to_string()),
        Err("No review is waiting for this agent pane".to_string()),
    );
}

#[rstest]
#[case::closes_the_displayed_review_and_remembers_it(
    Fake::new(&["e1", "e2"], false, Some("e2"), Some("e2")),
    outcome(&["hide", "labels:None:2"], &["e1", "e2"], None, Some("e2"), false),
)]
#[case::does_nothing_when_nothing_is_displayed(
    Fake::new(&["e1"], false, None, Some("e1")),
    outcome(&[], &["e1"], None, Some("e1"), false),
)]
fn hide_closes_the_displayed_review(#[case] runtime: Fake, #[case] expected: Outcome) {
    hide(&runtime, PARENT).expect("hide should succeed");

    assert_eq!(runtime.outcome(), expected);
}

#[rstest]
#[case::shows_when_nothing_is_displayed(
    Fake::new(&["e1"], false, None, None),
    Placement::Visible,
    outcome(&[], &["e1", "new"], Some("new"), Some("new"), false),
)]
#[case::replaces_crit(
    Fake::new(&[], true, Some("crit"), Some("crit")),
    Placement::Visible,
    outcome(&["hide"], &["new"], Some("new"), Some("new"), false),
)]
#[case::waits_behind_another_editor(
    Fake::new(&["e1"], true, Some("e1"), Some("e1")),
    Placement::Hidden,
    outcome(&[], &["e1", "new"], Some("e1"), Some("e1"), true),
)]
fn editor_started_decides_the_placement(
    #[case] runtime: Fake,
    #[case] placement: Placement,
    #[case] expected: Outcome,
) {
    let result = editor_started(&runtime, PARENT, Review::editor("new", "new"));

    assert_eq!(
        (result.map_err(|error| error.to_string()), runtime.outcome()),
        (Ok(placement), expected),
    );
}

#[rstest]
#[case::shows_the_next_review_when_the_displayed_one_ends(
    Fake::new(&["e1", "e2"], true, Some("e1"), Some("e1")),
    "e1",
    outcome(&["hide", "show:e2", "labels:Some(\"[e2] · crit\"):2"], &["e2"], Some("e2"), Some("e2"), true),
)]
#[case::wraps_to_the_first_review_when_the_last_ends(
    Fake::new(&["e1", "e2"], false, Some("e2"), Some("e2")),
    "e2",
    outcome(&["hide", "show:e1", "labels:Some(\"[e1]\"):1"], &["e1"], Some("e1"), Some("e1"), true),
)]
#[case::shows_crit_after_the_last_editor(
    Fake::new(&["e1"], true, Some("e1"), Some("e1")),
    "e1",
    outcome(&["hide", "show:crit", "labels:Some(\"[crit]\"):1"], &[], Some("crit"), Some("crit"), true),
)]
#[case::shows_an_editor_after_crit_ends(
    Fake::new(&["e1"], false, Some("crit"), Some("crit")),
    "crit",
    outcome(&["hide", "show:e1", "labels:Some(\"[e1]\"):1"], &["e1"], Some("e1"), Some("e1"), true),
)]
#[case::shows_nothing_when_the_last_review_ends(
    Fake::new(&["e1"], false, Some("e1"), Some("e1")),
    "e1",
    outcome(&["hide", "labels:None:0"], &[], None, None, false),
)]
#[case::keeps_a_hidden_state_when_a_hidden_review_ends(
    Fake::new(&["e1", "e2"], false, None, Some("e1")),
    "e2",
    outcome(&["labels:None:1"], &["e1"], None, Some("e1"), false),
)]
fn review_finished_shows_the_next_review(
    #[case] mut runtime: Fake,
    #[case] key: &str,
    #[case] expected: Outcome,
) {
    // The finished editor's session has exited.
    runtime.alive.retain(|alive| *alive != key);
    if key == CRIT_KEY {
        runtime.crit = false;
    }

    review_finished(&runtime, PARENT, key).expect("finish should succeed");

    assert_eq!(runtime.outcome(), expected);
}
