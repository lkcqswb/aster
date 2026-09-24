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
        if result["written"] == true {
            if !self.changed.iter().any(|p| p == subject) {
                self.changed.push(subject.into());
            }
            if let Some(diff) = result["diff"].as_str() {
                self.diffs
                    .push((subject.into(), crate::tools::clip(diff, 12000)));
            }
        }
        if matches!(name, "check_file" | "shell") && (error || result["passed"].is_boolean()) {
            let passed = !error && result["passed"].as_bool().unwrap_or(false);
            self.evidence.push(Evidence {
                label: format!("{name} · {}", crate::tools::clip(subject, 180)),
                passed,
                detail: crate::tools::clip(&result.to_string(), 4000),
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
        if self.evidence.iter().any(|e| !e.passed) {
            "Checks need attention"
        } else if !self.evidence.is_empty() {
            "Recorded checks passed"
        } else if !self.changed.is_empty() {
            "Changes need verification"
        } else {
            "No checks recorded"
        }
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
        for e in &self.evidence {
            out += &format!("{} {}\n", if e.passed { "✓" } else { "!" }, e.label);
        }
        if !self.changed.is_empty() {
            out += &format!("\nChanged files\n{}\n", self.changed.join("\n"));
        }
        out
    }
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
}
