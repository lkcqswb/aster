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
/// What exists right now, so the menu never offers a dead end.
#[derive(Clone, Debug, Default)]
pub struct Available {
    pub checkpoint: bool,
    pub waiting: usize,
    pub skills: usize,
    pub prompts: usize,
    pub entries: usize,
    pub messages: usize,
    pub plan_mode: bool,
}
pub struct Menu {
    pub items: Vec<Choice>,
    pub query: String,
    pub index: usize,
    pub available: Available,
}
impl Menu {
    pub fn new(work: &Work, running: bool, decision: bool, available: Available) -> Self {
        Self {
            items: choices(work, running, decision, &available),
            query: String::new(),
            index: 0,
            available,
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
        self.items = choices(work, running, decision, &self.available);
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
fn choices(work: &Work, running: bool, decision: bool, available: &Available) -> Vec<Choice> {
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
    if !work.goal.is_empty() {
        add(
            Action::Command("/work"),
            "Follow our plan",
            "Current step, activity and recorded evidence.",
        );
    }
    if !work.diffs.is_empty() {
        add(
            Action::Command("/review"),
            "Review this turn's changes",
            "Exact file diffs recorded during this task.",
        );
    }
    if !work.evidence.is_empty() && !(work.has_failures() || work.has_stale_checks()) {
        add(
            Action::Command("/checks"),
            "Check the evidence",
            "Actual checks and their history; not model claims.",
        );
    }
    add(
        Action::Command("/tasks"),
        "Run a project check or build",
        "Inspect named local commands before starting one.",
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
    if available.waiting > 0 {
        add(
            Action::Command("/queue"),
            "See waiting messages",
            "Saved directions and follow-up tasks.",
        );
    }
    if available.skills > 0 {
        add(
            Action::Command("/skills"),
            "Choose a skill",
            "Inspect resources and prepare an explicit request.",
        );
    }
    if available.prompts > 0 {
        add(
            Action::Command("/prompts"),
            "Choose a task prompt",
            "Prepare a reusable request before sending it.",
        );
    }
    if available.entries > 0 {
        add(
            Action::Command("/history"),
            "Find an earlier conversation entry",
            "Search what we said, read it or jump back to it.",
        );
    }
    add(
        Action::Command("/context"),
        "Inspect what I can see",
        "Context size, instructions, skills and attached references.",
    );
    if available.checkpoint {
        add(
            Action::Command("/checkpoint"),
            "Review our context checkpoint",
            "Read the summary and restore the full archived context.",
        );
    }
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
    } else {
        add(
            Action::Command("/sessions"),
            "Open another conversation",
            "Pick up where you left off in this project.",
        );
        add(
            Action::Command("/new"),
            "Start a new conversation",
            "This one is saved; project files are shared.",
        );
        if available.messages > 0 {
            add(
                Action::Command("/compact"),
                "Summarize earlier context",
                "Archive everything, keep recent exchanges and a summary.",
            );
        }
        if available.plan_mode {
            add(
                Action::Command("/build"),
                "Switch to build mode",
                "Allow file changes and commands, with your approval setting.",
            );
        } else {
            add(
                Action::Command("/plan"),
                "Switch to plan mode",
                "Read and discuss only; no file changes or commands.",
            );
        }
    }
    add(
        Action::Command("/help"),
        "Keys and commands",
        "Every shortcut and slash command in one place.",
    );
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_changes_remove_obsolete_actions_and_keep_the_filter() {
        let work = Work::default();
        let mut menu = Menu::new(&work, true, true, Available::default());
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
        // Dead ends are hidden until there is something behind them.
        assert!(!menu.items.iter().any(|c| matches!(
            c.action,
            Action::Command("/review" | "/checkpoint" | "/queue")
        )));
        let mut work = Work::default();
        work.diffs.push(("a.rs".into(), "+x".into()));
        let rich = Menu::new(
            &work,
            false,
            false,
            Available {
                checkpoint: true,
                waiting: 1,
                ..Default::default()
            },
        );
        for command in ["/review", "/checkpoint", "/queue", "/sessions", "/plan"] {
            assert!(
                rich.items
                    .iter()
                    .any(|c| c.action == Action::Command(command)),
                "{command}"
            );
        }
        menu.query.clear();
        menu.paste("files");
        assert_eq!(menu.filtered()[0].action, Action::Command("/files"));
    }
}
