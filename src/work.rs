use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Doing,
    Done,
    Blocked,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Step {
    pub title: String,
    pub status: StepStatus,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Evidence {
    pub label: String,
    pub passed: bool,
    pub detail: String,
    #[serde(default)]
    pub identity: String,
    #[serde(default)]
    pub revision: u64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EvidenceState {
    Passed,
    Failed,
    Stale,
    Earlier,
}
impl EvidenceState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "needs attention",
            Self::Stale => "rerun after edits",
            Self::Earlier => "earlier result",
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Work {
    pub goal: String,
    pub steps: Vec<Step>,
    pub activity: String,
    pub focus: String,
    pub changed: Vec<String>,
    pub diffs: Vec<(String, String)>,
    pub evidence: Vec<Evidence>,
    pub waiting: String,
    pub skills: Vec<String>,
    pub context_files: Vec<String>,
    pub command: Option<crate::tools::CommandProgress>,
    pub model_requests: u64,
    pub revision: u64,
}
impl Work {
    pub fn begin(prompt: &str) -> Self {
        Self {
            goal: crate::tools::clip(prompt, 400),
            activity: "Understanding the task".into(),
            ..Default::default()
        }
    }
    pub fn set_plan(&mut self, value: &Value) -> Result<()> {
        let steps: Vec<Step> = serde_json::from_value(value.clone())?;
        if steps.is_empty()
            || steps.len() > 12
            || steps
                .iter()
                .any(|s| s.title.trim().is_empty() || s.title.len() > 180)
        {
            bail!("Use 1–12 steps with titles of 1–180 bytes");
        }
        if steps
            .iter()
            .filter(|s| s.status == StepStatus::Doing)
            .count()
            > 1
        {
            bail!("Only one step can be doing at a time");
        }
        self.steps = steps;
        Ok(())
    }
    pub fn record(&mut self, name: &str, args: &Value, result: &Value, error: bool) {
        let subject = args["path"]
            .as_str()
            .or(args["command"].as_str())
            .unwrap_or(name);
        if args["path"].is_string() || args["command"].is_string() {
            self.focus = crate::tools::clip(subject, 240);
        }
        self.waiting.clear();
        if name == "read_skill"
            && !error
            && let Some(skill) = result["skill"].as_str()
        {
            if !self.skills.iter().any(|s| s == skill) {
                self.skills.push(skill.into());
            }
            self.focus = format!("Skill · {skill}");
        }
        if result["written"] == true {
            self.revision = self.revision.saturating_add(1);
            if !self.changed.iter().any(|p| p == subject) {
                self.changed.push(subject.into());
            }
            if let Some(diff) = result["diff"].as_str() {
                self.diffs
                    .push((subject.into(), crate::tools::clip(diff, 12000)));
            }
        }
        if matches!(name, "check_file" | "shell")
            && result["executed"] != false
            && (error || result["passed"].is_boolean())
        {
            let passed = !error && result["passed"].as_bool().unwrap_or(false);
            self.evidence.push(Evidence {
                label: format!(
                    "{} · {}",
                    if name == "shell" {
                        "Command"
                    } else {
                        "File check"
                    },
                    crate::tools::clip(subject, 180)
                ),
                passed,
                detail: readable_detail(result),
                identity: check_identity(name, args),
                revision: self.revision,
            });
        }
        self.activity = if error {
            "An action needs attention"
        } else if result["passed"] == false {
            "A check failed"
        } else {
            "Continuing the task"
        }
        .into();
    }
    pub fn verdict(&self) -> &'static str {
        if self.has_failures() {
            "Checks need attention"
        } else if self.has_stale_checks() {
            "Checks need rerunning after edits"
        } else if !self.evidence.is_empty() {
            "Recorded checks passed"
        } else if !self.changed.is_empty() {
            "Changes need verification"
        } else {
            "No checks recorded"
        }
    }
    pub fn evidence_state(&self, index: usize) -> EvidenceState {
        let evidence = &self.evidence[index];
        if !evidence.identity.is_empty()
            && self.evidence[index + 1..]
                .iter()
                .any(|later| later.identity == evidence.identity)
        {
            EvidenceState::Earlier
        } else if !evidence.passed {
            EvidenceState::Failed
        } else if evidence.revision < self.revision {
            EvidenceState::Stale
        } else {
            EvidenceState::Passed
        }
    }
    pub fn has_failures(&self) -> bool {
        (0..self.evidence.len()).any(|i| self.evidence_state(i) == EvidenceState::Failed)
    }
    pub fn has_stale_checks(&self) -> bool {
        (0..self.evidence.len()).any(|i| self.evidence_state(i) == EvidenceState::Stale)
    }
    pub fn verified(&self) -> bool {
        !self.evidence.is_empty() && !self.has_failures() && !self.has_stale_checks()
    }
    pub fn checks_summary(&self) -> String {
        if self.evidence.is_empty() {
            return "No checks recorded for this turn.\n\n/check PATH [expected JSON] verifies a file.\n/run COMMAND runs a local check with your permission settings.".into();
        }
        let mut out = format!("{}\n\n", self.verdict());
        for (index, evidence) in self.evidence.iter().enumerate() {
            out += &format!(
                "{}. {} · {}\nOriginal outcome: {}\n{}\n\n",
                index + 1,
                evidence.label,
                self.evidence_state(index).label(),
                if evidence.passed { "passed" } else { "failed" },
                serde_json::from_str::<Value>(&evidence.detail)
                    .map(|value| readable_detail(&value))
                    .unwrap_or_else(|_| evidence.detail.clone())
            );
        }
        out += "Earlier results remain in history. Only the same check can replace its result.\nFile edits make previous passing results stale; rerun the relevant checks.\nArbitrary shell or external edits are not automatically tracked.";
        out
    }
    pub fn summary(&self) -> String {
        let mut out = format!("{}\n\n{}\n{}\n\n", self.goal, self.activity, self.focus);
        for (i, step) in self.steps.iter().enumerate() {
            let mark = match step.status {
                StepStatus::Pending => "○",
                StepStatus::Doing => "›",
                StepStatus::Done => "✓",
                StepStatus::Blocked => "!",
            };
            out += &format!("{mark} {}. {}\n", i + 1, step.title);
        }
        out += &format!("\n{}\n", self.verdict());
        for (index, e) in self.evidence.iter().enumerate() {
            let state = self.evidence_state(index);
            out += &format!(
                "{} {} · {}\n",
                match state {
                    EvidenceState::Passed => "✓",
                    EvidenceState::Failed => "!",
                    EvidenceState::Stale => "↻",
                    EvidenceState::Earlier => "·",
                },
                e.label,
                state.label()
            );
        }
        if !self.changed.is_empty() {
            out += &format!("\nChanged files\n{}\n", self.changed.join("\n"));
        }
        if !self.skills.is_empty() {
            out += &format!("\nSkills in use\n{}\n", self.skills.join("\n"));
        }
        if !self.context_files.is_empty() {
            out += &format!("\nAttached files\n{}\n", self.context_files.join("\n"));
        }
        if let Some(command) = &self.command {
            out += &format!(
                "\nLatest command\n$ {}\n{:.1}s · {} output bytes · {}\nF4 /output opens its output.\n",
                command.command,
                command.elapsed_ms as f64 / 1000.,
                command.total_bytes,
                if command.running {
                    "running"
                } else if command.timed_out {
                    "timed out"
                } else if command.stopped {
                    "stopped"
                } else if command.exit_code == Some(0) {
                    "exit 0"
                } else {
                    "failed"
                }
            );
        }
        out
    }
}

fn check_identity(name: &str, args: &Value) -> String {
    if name == "shell" {
        return serde_json::json!([name, args["command"]]).to_string();
    }
    let expected = if args["kind"] == "json_equals" {
        args["expected"]
            .as_str()
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .unwrap_or_else(|| args["expected"].clone())
    } else {
        args["expected"].clone()
    };
    serde_json::json!([name, args["path"], args["kind"], expected]).to_string()
}

fn readable_detail(value: &Value) -> String {
    if let Some(error) = value["error"].as_str() {
        return format!(
            "Could not complete the check: {}",
            crate::tools::clip(error, 1200)
        );
    }
    if let Some(kind) = value["kind"].as_str() {
        let assertion = match kind {
            "exists" => "File exists",
            "contains" => "Contains text",
            "text_equals" => "Exact text",
            "json_equals" => "Exact JSON",
            _ => kind,
        };
        return if kind == "exists" {
            assertion.into()
        } else {
            format!(
                "{assertion}: {}",
                crate::tools::clip(&value["expected"].to_string(), 1600)
            )
        };
    }
    if value.get("exit_code").is_some() {
        let status = if value["stopped"] == true {
            "Stopped by you".into()
        } else if value["timed_out"] == true {
            "Timed out".into()
        } else {
            format!("Exit {}", value["exit_code"])
        };
        let mut detail = format!(
            "{status} · {:.1}s",
            value["duration_ms"].as_u64().unwrap_or(0) as f64 / 1000.
        );
        for (field, label) in [("stdout", "Output"), ("stderr", "Errors")] {
            if let Some(text) = value[field].as_str().filter(|text| !text.trim().is_empty()) {
                detail += &format!("\n{label}: {}", crate::tools::clip(text.trim(), 1200));
            }
        }
        return detail;
    }
    value["detail"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| "Outcome recorded by the tool".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn plan_claims_do_not_create_evidence_or_erase_failures() {
        let mut w = Work::begin("repair");
        w.set_plan(&json!([{"title":"fix","status":"done"}]))
            .unwrap();
        assert_eq!(w.verdict(), "No checks recorded");
        w.record(
            "check_file",
            &json!({"path":"a"}),
            &json!({"passed":false}),
            false,
        );
        w.record(
            "check_file",
            &json!({"path":"b"}),
            &json!({"passed":true}),
            false,
        );
        assert_eq!(w.verdict(), "Checks need attention");
        assert_eq!(Work::begin("next").verdict(), "No checks recorded");
    }
    #[test]
    fn unexecuted_checks_are_not_failed_tests() {
        let mut w = Work::begin("test");
        w.record(
            "shell",
            &json!({"command":"cargo test"}),
            &json!({"error":"declined","executed":false}),
            true,
        );
        assert!(w.evidence.is_empty());
        w.record(
            "check_file",
            &json!({"path":"bad.json"}),
            &json!({"error":"invalid json","executed":true}),
            true,
        );
        assert_eq!(w.evidence.len(), 1);
        assert!(!w.evidence[0].passed);
    }
    #[test]
    fn same_check_can_recover_without_erasing_the_failed_result() {
        let mut work = Work::begin("repair");
        let args = json!({"path":"result.json","kind":"json_equals","expected":"{\"ok\":true}"});
        work.record("check_file", &args, &json!({"passed":false}), false);
        work.record(
            "edit_file",
            &json!({"path":"result.json"}),
            &json!({"written":true}),
            false,
        );
        work.record("check_file", &args, &json!({"passed":true}), false);
        assert!(work.verified());
        assert_eq!(work.evidence.len(), 2);
        assert!(!work.evidence[0].passed);
        assert_eq!(work.evidence_state(0), EvidenceState::Earlier);
        assert!(work.checks_summary().contains("Original outcome: failed"));
    }
    #[test]
    fn changing_an_assertion_cannot_resolve_a_failure() {
        let mut work = Work::begin("repair");
        work.record(
            "check_file",
            &json!({"path":"result.json","kind":"json_equals","expected":"{\"ok\":true}"}),
            &json!({"passed":false}),
            false,
        );
        work.record(
            "check_file",
            &json!({"path":"result.json","kind":"exists","expected":""}),
            &json!({"passed":true}),
            false,
        );
        assert!(work.has_failures());
        assert!(!work.verified());
        assert_eq!(work.evidence_state(0), EvidenceState::Failed);
    }
    #[test]
    fn edits_invalidate_passing_checks_until_they_are_rerun() {
        let mut work = Work::begin("repair");
        let args = json!({"command":"cargo test","timeout_secs":20});
        work.record("shell", &args, &json!({"passed":true}), false);
        work.record(
            "edit_file",
            &json!({"path":"src/lib.rs"}),
            &json!({"written":true}),
            false,
        );
        assert_eq!(work.verdict(), "Checks need rerunning after edits");
        assert!(!work.verified());
        work.record(
            "shell",
            &json!({"command":"cargo test","timeout_secs":60}),
            &json!({"passed":true}),
            false,
        );
        assert!(work.verified());
        assert_eq!(work.evidence_state(0), EvidenceState::Earlier);
    }
    #[test]
    fn a_later_failure_replaces_a_pass_and_legacy_history_remains_readable() {
        let mut work: Work = serde_json::from_value(
            json!({"evidence":[{"label":"old","passed":true,"detail":"saved before revisions"}]}),
        )
        .unwrap();
        assert!(work.verified());
        let args = json!({"command":"cargo test"});
        work.record("shell", &args, &json!({"passed":true}), false);
        work.record("shell", &args, &json!({"passed":false}), false);
        assert!(work.has_failures());
        assert_eq!(work.evidence_state(0), EvidenceState::Passed);
        assert_eq!(work.evidence_state(1), EvidenceState::Earlier);
        let saved = serde_json::to_value(&work).unwrap();
        let restored: Work = serde_json::from_value(saved).unwrap();
        assert_eq!(restored.verdict(), work.verdict());
    }
}
