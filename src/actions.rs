//! Local companion controls. Opening or filtering this menu never calls a model.
use crate::work::Work;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Return,
    Command(&'static str),
    Redirect,
    Stop,
}
#[derive(Clone)]
pub struct Choice {
    pub action: Action,
    pub label: &'static str,
    pub detail: &'static str,
}
pub struct Menu {
    pub items: Vec<Choice>,
    pub query: String,
    pub index: usize,
}
impl Menu {
    pub fn new(work: &Work, running: bool, decision: bool) -> Self {
        Self {
            items: choices(work, running, decision),
            query: String::new(),
            index: 0,
        }
    }
    pub fn filtered(&self) -> Vec<&Choice> {
        let query = self.query.to_lowercase();
        self.items
            .iter()
            .filter(|c| {
                format!("{} {} {:?}", c.label, c.detail, c.action)
                    .to_lowercase()
                    .contains(&query)
            })
            .collect()
    }
    pub fn refresh(&mut self, work: &Work, running: bool, decision: bool) {
        let selected = self.filtered().get(self.index).map(|c| c.action);
        self.items = choices(work, running, decision);
        self.index = self
            .filtered()
            .iter()
            .position(|c| Some(c.action) == selected)
            .unwrap_or(0);
    }
    pub fn paste(&mut self, text: &str) {
        let text = text.replace(['\n', '\r'], " ");
        if self.query.len() + text.len() <= 160 {
            self.query.push_str(&text);
            self.index = 0;
        }
    }
}
fn choices(work: &Work, running: bool, decision: bool) -> Vec<Choice> {
    let mut items = vec![];
    let mut add = |action, label, detail| {
        items.push(Choice {
            action,
            label,
            detail,
        })
    };
    if decision {
        add(
            Action::Return,
            "Back to your decision",
            "Your answer or approval is still waiting.",
        );
    }
    if work.command.is_some() {
        add(
            Action::Command("/output"),
            "Read command output",
            "Live output, errors and the actual exit status.",
        );
    }
    if work.has_failures() || work.has_stale_checks() {
        add(
            Action::Command("/checks"),
            "Inspect checks needing attention",
            "See failures, stale passes and earlier results.",
        );
    }
    add(
        Action::Command("/work"),
        "Follow our plan",
        "Current step, activity and recorded evidence.",
    );
    add(
        Action::Command("/files"),
        "Read project files together",
        "Browse, preview and attach source to your draft.",
    );
    add(
        Action::Command("/find"),
        "Find something in the project",
        "Search locally and read the matching lines.",
    );
    add(
        Action::Command("/review"),
        "Review this turn's changes",
        "Exact file diffs recorded during this task.",
    );
    if !(work.has_failures() || work.has_stale_checks()) {
        add(
            Action::Command("/checks"),
            "Check the evidence",
            "Actual checks and their history; not model claims.",
        );
    }
    add(
        Action::Command("/context"),
        "Inspect what I can see",
        "Instructions, used skills and attached references.",
    );
    add(
        Action::Command("/checkpoint"),
        "Review our context checkpoint",
        "Inspect retained history and restore full context.",
    );
    add(
        Action::Command("/skills"),
        "Choose a skill",
        "Inspect resources and prepare an explicit request.",
    );
    add(
        Action::Command("/prompts"),
        "Choose a task prompt",
        "Prepare a reusable request before sending it.",
    );
    add(
        Action::Command("/queue"),
        "See waiting messages",
        "Saved directions and follow-up tasks.",
    );
    if running {
        add(
            Action::Redirect,
            "Change our direction",
            "Type a correction; send it to cancel pending actions.",
        );
        add(
            Action::Stop,
            "Stop the current task",
            "Cancel work and keep saved follow-up messages.",
        );
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_changes_remove_obsolete_actions_and_keep_the_filter() {
        let work = Work::default();
        let mut menu = Menu::new(&work, true, true);
        assert_eq!(menu.items[0].action, Action::Return);
        menu.paste("stop");
        assert_eq!(menu.filtered()[0].action, Action::Stop);
        menu.refresh(&work, false, false);
        assert!(menu.filtered().is_empty());
        assert_eq!(menu.query, "stop");
        assert!(
            !menu
                .items
                .iter()
                .any(|c| matches!(c.action, Action::Return | Action::Stop | Action::Redirect))
        );
        menu.query.clear();
        menu.paste("files");
        assert_eq!(menu.filtered()[0].action, Action::Command("/files"));
    }
}
