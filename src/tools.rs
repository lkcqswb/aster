use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub fn schemas() -> Value {
    json!([
     {"name":"update_plan","description":"Share or update a concise work plan for a multi-step task. One step may be doing. Marking a step done is a progress statement, never verification evidence.","input_schema":{"type":"object","properties":{"steps":{"type":"array","minItems":1,"maxItems":12,"items":{"type":"object","properties":{"title":{"type":"string"},"status":{"type":"string","enum":["pending","doing","done","blocked"]}},"required":["title","status"],"additionalProperties":false}}},"required":["steps"],"additionalProperties":false}},
     {"name":"ask_user","description":"Ask one necessary, actionable question during work. Provide 2–5 short options when useful; the user may instead write an answer. Wait for the answer before dependent work. Do not use this for tool approvals.","input_schema":{"type":"object","properties":{"question":{"type":"string"},"options":{"type":"array","maxItems":5,"items":{"type":"string"}}},"required":["question","options"],"additionalProperties":false}},
     {"name":"read_skill","description":"Load an available skill or a supporting text file. Skills are listed by name in the system context. Read SKILL.md first; optional path is relative to the skill directory. Loading a skill grants no tool permissions.","input_schema":{"type":"object","properties":{"name":{"type":"string"},"path":{"type":"string"}},"required":["name"],"additionalProperties":false}},
     {"name":"list_files","description":"List project files, excluding credentials and generated/private directories.","input_schema":{"type":"object","properties":{},"additionalProperties":false}},
     {"name":"read_file","description":"Read a UTF-8 project file with line numbers. offset is a one-based line; limit is 1–500 lines (default 200). Use next_offset to continue. Nested AGENTS.md guidance is provided before access.","input_schema":{"type":"object","properties":{"path":{"type":"string","description":"Path relative to the project root."},"offset":{"type":"integer","minimum":1},"limit":{"type":"integer","minimum":1,"maximum":500}},"required":["path"],"additionalProperties":false}},
     {"name":"search","description":"Find a literal string in project text files. Bounded to 100 matching lines.","input_schema":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false}},
     {"name":"write_file","description":"Create or replace a UTF-8 project file. Requires user permission in ask mode. Respect AGENTS.md.","input_schema":{"type":"object","properties":{"path":{"type":"string","description":"Relative to the project root, such as src/main.rs. Never an absolute path."},"content":{"type":"string"}},"required":["path","content"],"additionalProperties":false}},
     {"name":"edit_file","description":"Replace one exact, unique old_text occurrence in a UTF-8 project file. Read the file first, include enough context to match once, and preserve unrelated content. Shows a diff for approval and rejects stale edits.","input_schema":{"type":"object","properties":{"path":{"type":"string"},"old_text":{"type":"string"},"new_text":{"type":"string"}},"required":["path","old_text","new_text"],"additionalProperties":false}},
     {"name":"shell","description":"Run a shell command in the project with streamed output and a bounded timeout (default 30 seconds, maximum 120). Requires explicit permission in ask mode. It is NOT a filesystem sandbox.","input_schema":{"type":"object","properties":{"command":{"type":"string"},"timeout_secs":{"type":"integer","minimum":1,"maximum":120}},"required":["command"],"additionalProperties":false}},
     {"name":"check_file","description":"Independently verify a saved file. kind is exists, contains, text_equals or json_equals. expected is a STRING: serialized JSON for json_equals, literal text for text checks, or empty for exists. A check is not a model opinion.","input_schema":{"type":"object","properties":{"path":{"type":"string","description":"Relative to the project root, such as src/main.rs. Never an absolute path."},"kind":{"type":"string","enum":["exists","contains","text_equals","json_equals"]},"expected":{"type":"string","description":"For json_equals, a JSON-encoded value as text, e.g. {\"ready\":true}. For exists, use an empty string."}},"required":["path","kind","expected"],"additionalProperties":false}}
    ])
}
pub fn sensitive(name: &str) -> bool {
    name == ".env"
        || (name.starts_with(".env.") && name != ".env.example")
        || matches!(
            name,
            ".git"
                | ".aster"
                | ".relay"
                | ".venv"
                | "node_modules"
                | "target"
                | ".ssh"
                | ".aws"
                | ".codex"
                | ".claude"
                | ".config"
                | ".local"
                | "__pycache__"
        )
        || name.ends_with(".pem")
        || name.ends_with(".key")
}
pub fn path(root: &Path, name: &str) -> Result<PathBuf> {
    if name.is_empty() || name.len() > 1024 {
        bail!("Use a nonempty relative path")
    }
    let relative = Path::new(name);
    let mut out = root.to_path_buf();
    for part in relative.components() {
        match part {
            Component::Normal(s) => {
                let s = s.to_string_lossy();
                if sensitive(&s) {
                    bail!("Credential/private path is excluded")
                };
                out.push(s.as_ref());
                if out
                    .symlink_metadata()
                    .is_ok_and(|m| m.file_type().is_symlink())
                {
                    bail!("Symlink access is not supported")
                }
            }
            Component::CurDir => {}
            _ => bail!("Path must remain in the project"),
        }
    }
    if out == root {
        bail!("Name a project file")
    };
    Ok(out)
}
pub fn files(root: &Path) -> Result<Vec<String>> {
    fn visit(root: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) -> Result<()> {
        if depth > 6 || out.len() >= 800 {
            return Ok(());
        }
        let mut entries = fs::read_dir(dir)?
            .filter_map(|e| e.ok())
            .collect::<Vec<_>>();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            if out.len() >= 800 {
                break;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if sensitive(&name) {
                continue;
            }
            let kind = e.file_type()?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                visit(root, &e.path(), depth + 1, out)?
            } else if kind.is_file() {
                out.push(e.path().strip_prefix(root)?.to_string_lossy().into_owned());
            }
        }
        Ok(())
    }
    let mut out = vec![];
    visit(root, root, 0, &mut out)?;
    Ok(out)
}
fn string<'a>(a: &'a Value, k: &str) -> Result<&'a str> {
    a[k].as_str()
        .with_context(|| format!("{k} must be a string"))
}
pub fn mutates(name: &str) -> bool {
    matches!(name, "write_file" | "edit_file" | "shell")
}
pub fn preview(root: &Path, name: &str, a: &Value) -> String {
    if name == "shell" {
        return format!(
            "$ {}\n\nUp to {} seconds, within the active turn budget.\nRuns in {}. Shell access is not sandboxed.",
            a["command"].as_str().unwrap_or(""),
            a["timeout_secs"].as_u64().unwrap_or(30),
            root.display()
        );
    }
    crate::edits::prepare(root, name, a)
        .map(|edit| clip(&edit.diff, 12000))
        .unwrap_or_else(|e| e.to_string())
}
pub fn clip(s: &str, n: usize) -> String {
    let mut end = s.len().min(n);
    while !s.is_char_boundary(end) {
        end -= 1
    }
    if end < s.len() {
        format!("{}\n… [truncated]", &s[..end])
    } else {
        s.into()
    }
}
fn read(root: &Path, name: &str) -> Result<String> {
    let p = path(root, name)?;
    if fs::metadata(&p)?.len() > 128_000 {
        bail!("File exceeds 128 KB")
    };
    Ok(fs::read_to_string(p)?)
}
pub fn validate(name: &str, a: &Value) -> Result<()> {
    let spec = schemas()
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == name)
        .cloned()
        .context("Unknown tool")?;
    let obj = a.as_object().context("Tool input must be an object")?;
    let props = spec["input_schema"]["properties"].as_object().unwrap();
    if obj.keys().any(|k| !props.contains_key(k))
        || spec["input_schema"]["required"]
            .as_array()
            .is_some_and(|req| req.iter().any(|k| !obj.contains_key(k.as_str().unwrap())))
    {
        bail!("Invalid tool arguments")
    }
    Ok(())
}
pub fn execute(root: &Path, name: &str, a: &Value, cancel: &Arc<AtomicBool>) -> Result<Value> {
    execute_with_progress(root, name, a, cancel, &mut |_| {})
}
pub fn execute_with_progress(
    root: &Path,
    name: &str,
    a: &Value,
    cancel: &Arc<AtomicBool>,
    progress: &mut dyn FnMut(&CommandProgress),
) -> Result<Value> {
    if cancel.load(Ordering::Relaxed) {
        bail!("Stopped");
    }
    validate(name, a)?;
    match name {
        "list_files" => Ok(json!({"files":files(root)?,"limit":800})),
        "read_file" => {
            let offset = a
                .get("offset")
                .map(|v| v.as_u64().context("offset must be a positive integer"))
                .transpose()?
                .unwrap_or(1) as usize;
            let limit = a
                .get("limit")
                .map(|v| v.as_u64().context("limit must be an integer"))
                .transpose()?
                .unwrap_or(200) as usize;
            if offset == 0 || !(1..=500).contains(&limit) {
                bail!("Use offset >= 1 and limit between 1 and 500");
            }
            let text = read(root, string(a, "path")?)?;
            let lines: Vec<_> = text.lines().collect();
            let selected: Vec<_> = lines
                .iter()
                .enumerate()
                .skip(offset - 1)
                .take(limit)
                .collect();
            let content = selected
                .iter()
                .map(|(i, t)| format!("{}: {t}", i + 1))
                .collect::<Vec<_>>()
                .join("\n");
            let end = (offset - 1).saturating_add(selected.len());
            Ok(
                json!({"path":string(a,"path")?,"content":content,"offset":offset,"total_lines":lines.len(),"next_offset":if end < lines.len(){Some(end+1)}else{None}}),
            )
        }
        "search" => {
            let q = string(a, "query")?;
            if q.is_empty() || q.len() > 500 {
                bail!("Search needs 1–500 bytes")
            };
            let mut matches = vec![];
            for p in files(root)? {
                if let Ok(text) = read(root, &p) {
                    for (i, line) in text.lines().enumerate() {
                        if line.contains(q) {
                            matches.push(json!({"path":p,"line":i+1,"text":clip(line,300)}));
                            if matches.len() >= 100 {
                                return Ok(json!({"matches":matches,"truncated":true}));
                            }
                        }
                    }
                }
            }
            Ok(json!({"matches":matches}))
        }
        "write_file" | "edit_file" => crate::edits::prepare(root, name, a)?.commit(root),
        "check_file" => {
            let kind = string(a, "kind")?;
            let filename = string(a, "path")?;
            let p = path(root, filename)?;
            let expected = if kind == "json_equals" {
                serde_json::from_str::<Value>(string(a, "expected")?)
                    .context("expected must be a JSON-encoded value as text")?
            } else {
                a["expected"].clone()
            };
            let passed = if kind == "exists" {
                p.is_file()
            } else {
                let text = read(root, filename)?;
                match kind {
                    "contains" => {
                        text.contains(a["expected"].as_str().context("expected must be text")?)
                    }
                    "text_equals" => {
                        text == a["expected"].as_str().context("expected must be text")?
                    }
                    "json_equals" => serde_json::from_str::<Value>(&text)? == expected,
                    _ => bail!("Unknown check type"),
                }
            };
            Ok(
                json!({"path":filename,"kind":kind,"expected":expected,"passed":passed,"detail":if passed{"Independent file check passed"}else{"Independent file check failed"}}),
            )
        }
        "shell" => shell(
            root,
            string(a, "command")?,
            shell_timeout(a)?,
            cancel,
            progress,
        ),
        _ => bail!("Unknown tool"),
    }
}
pub fn shell_timeout(args: &Value) -> Result<u64> {
    let timeout = args
        .get("timeout_secs")
        .map(|v| v.as_u64().context("timeout_secs must be an integer"))
        .transpose()?
        .unwrap_or(30);
    if !(1..=120).contains(&timeout) {
        bail!("Command timeout must be 1–120 seconds");
    }
    Ok(timeout)
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CommandProgress {
    pub command: String,
    pub elapsed_ms: u64,
    pub timeout_secs: u64,
    pub stdout_tail: String,
    pub stderr_tail: String,
    pub total_bytes: usize,
    pub truncated: bool,
    pub running: bool,
    pub exit_code: Option<i32>,
    pub stopped: bool,
    pub timed_out: bool,
}
impl CommandProgress {
    pub fn summary(&self) -> String {
        let status = if self.running {
            "Running"
        } else if self.timed_out {
            "Timed out"
        } else if self.stopped {
            "Stopped by you"
        } else if self.exit_code == Some(0) {
            "Finished · exit 0"
        } else {
            "Failed"
        };
        format!(
            "$ {}\n\n{} · {:.1}s / {}s · {} output bytes\nExit: {}{}\n\nSTDOUT (latest output)\n{}\n\nSTDERR (latest output)\n{}\n\nEsc closes this panel; Esc again stops running work.\nCtrl+G changes direction; a running command finishes first.",
            self.command,
            status,
            self.elapsed_ms as f64 / 1000.,
            self.timeout_secs,
            self.total_bytes,
            self.exit_code.map_or("pending".into(), |c| c.to_string()),
            if self.truncated {
                " · full result clipped"
            } else {
                ""
            },
            self.stdout_tail,
            self.stderr_tail
        )
    }
}
#[derive(Default)]
struct Capture {
    bytes: Vec<u8>,
    total: usize,
}
impl Capture {
    fn push(&mut self, chunk: &[u8]) {
        self.total = self.total.saturating_add(chunk.len());
        self.bytes.extend_from_slice(chunk);
        if self.bytes.len() > 32_000 {
            self.bytes.drain(16_000..self.bytes.len() - 16_000);
        }
    }
    fn tail(&self) -> String {
        String::from_utf8_lossy(&self.bytes[self.bytes.len().saturating_sub(4000)..]).into_owned()
    }
    fn text(&self) -> String {
        if self.total <= 32_000 {
            String::from_utf8_lossy(&self.bytes).into_owned()
        } else {
            format!(
                "{}\n… [output truncated: first and last 16 KB retained] …\n{}",
                String::from_utf8_lossy(&self.bytes[..16_000]),
                String::from_utf8_lossy(&self.bytes[16_000..])
            )
        }
    }
}
struct ProcessGroup {
    child: std::process::Child,
    cleaned: bool,
}
impl ProcessGroup {
    fn kill(&mut self) {
        if self.cleaned {
            return;
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.cleaned = true;
    }
}
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.kill();
    }
}
fn shell(
    root: &Path,
    command: &str,
    timeout_secs: u64,
    cancel: &Arc<AtomicBool>,
    progress: &mut dyn FnMut(&CommandProgress),
) -> Result<Value> {
    if command.len() > 8000 || command.trim().is_empty() {
        bail!("Use a nonempty command up to 8 KB");
    }
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, _) in std::env::vars() {
        let k = key.to_uppercase();
        if k.contains("TOKEN")
            || k.contains("SECRET")
            || k.contains("PASSWORD")
            || k.ends_with("_KEY")
        {
            cmd.env_remove(key);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut process = ProcessGroup {
        child: cmd.spawn()?,
        cleaned: false,
    };
    let stdout = Arc::new(Mutex::new(Capture::default()));
    let stderr = Arc::new(Mutex::new(Capture::default()));
    let drain = |mut stream: Box<dyn Read + Send>, capture: Arc<Mutex<Capture>>| {
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = stream.read(&mut buf) {
                if n == 0 {
                    break;
                }
                capture.lock().unwrap().push(&buf[..n]);
            }
        })
    };
    let out = drain(
        Box::new(process.child.stdout.take().unwrap()),
        stdout.clone(),
    );
    let err = drain(
        Box::new(process.child.stderr.take().unwrap()),
        stderr.clone(),
    );
    let start = Instant::now();
    let mut state = CommandProgress {
        command: command.into(),
        timeout_secs,
        running: true,
        ..Default::default()
    };
    let mut updated = start - Duration::from_secs(1);
    let mut refresh = |state: &mut CommandProgress| {
        let out = stdout.lock().unwrap();
        let err = stderr.lock().unwrap();
        state.elapsed_ms = start.elapsed().as_millis() as u64;
        state.stdout_tail = out.tail();
        state.stderr_tail = err.tail();
        state.total_bytes = out.total.saturating_add(err.total);
        state.truncated = out.total > 32_000 || err.total > 32_000;
        progress(state);
    };
    let status = loop {
        if updated.elapsed() >= Duration::from_millis(200) {
            refresh(&mut state);
            updated = Instant::now();
        }
        if cancel.load(Ordering::Relaxed) || start.elapsed() >= Duration::from_secs(timeout_secs) {
            state.stopped = cancel.load(Ordering::Relaxed);
            state.timed_out = !state.stopped;
            process.kill();
            break process.child.wait()?;
        }
        if let Some(status) = process.child.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_millis(40));
    };
    process.kill();
    let _ = out.join();
    let _ = err.join();
    state.running = false;
    state.exit_code = status.code();
    refresh(&mut state);
    Ok(
        json!({"command":command,"exit_code":status.code(),"stdout":stdout.lock().unwrap().text(),"stderr":stderr.lock().unwrap().text(),"stopped":state.stopped,"timed_out":state.timed_out,"duration_ms":state.elapsed_ms,"output_bytes":state.total_bytes,"output_truncated":state.truncated,"passed":status.success()&&!state.stopped&&!state.timed_out}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paginated_read_reports_continuation_and_bounds() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a"), "first\nsecond\nthird\n").unwrap();
        let c = Arc::new(AtomicBool::new(false));
        let a = execute(
            d.path(),
            "read_file",
            &json!({"path":"a","offset":1,"limit":2}),
            &c,
        )
        .unwrap();
        assert_eq!(a["content"], "1: first\n2: second");
        assert_eq!(a["next_offset"], 3);
        let b = execute(
            d.path(),
            "read_file",
            &json!({"path":"a","offset":3,"limit":2}),
            &c,
        )
        .unwrap();
        assert_eq!(b["content"], "3: third");
        assert!(b["next_offset"].is_null());
        assert!(execute(d.path(), "read_file", &json!({"path":"a","offset":0}), &c).is_err());
    }
    #[test]
    fn boundaries_and_exact_check() {
        let d = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        for p in [
            "../x",
            "/tmp/x",
            ".env",
            ".git/config",
            "nested/.ssh/id_rsa",
        ] {
            assert!(path(d.path(), p).is_err())
        }
        execute(
            d.path(),
            "write_file",
            &json!({"path":"a.json","content":"{\"v\":true}"}),
            &cancel,
        )
        .unwrap();
        assert_eq!(
            execute(
                d.path(),
                "check_file",
                &json!({"path":"a.json","kind":"json_equals","expected":"{\"v\":1}"}),
                &cancel
            )
            .unwrap()["passed"],
            false
        );
        cancel.store(true, Ordering::Relaxed);
        assert!(
            execute(
                d.path(),
                "write_file",
                &json!({"path":"b","content":"bad"}),
                &cancel
            )
            .is_err()
        );
    }
    #[cfg(unix)]
    #[test]
    fn symlink_escape() {
        let d = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/tmp", d.path().join("out")).unwrap();
        assert!(path(d.path(), "out/test").is_err());
    }
    #[test]
    fn shell_exit_is_not_a_claim() {
        let d = tempfile::tempdir().unwrap();
        let c = Arc::new(AtomicBool::new(false));
        let r = execute(
            d.path(),
            "shell",
            &json!({"command":"printf test; exit 7"}),
            &c,
        )
        .unwrap();
        assert_eq!(r["exit_code"], 7);
        assert_eq!(r["passed"], false);
        assert_eq!(r["stdout"], "test");
    }
    #[test]
    fn shell_streams_before_exit_and_retains_final_failure() {
        let d = tempfile::tempdir().unwrap();
        let mut updates = vec![];
        let r = execute_with_progress(
            d.path(),
            "shell",
            &json!({"command":"printf early; sleep .3; printf late; printf error >&2; exit 7"}),
            &Arc::new(AtomicBool::new(false)),
            &mut |p| updates.push(p.clone()),
        )
        .unwrap();
        assert!(
            updates
                .iter()
                .any(|p| p.running && p.stdout_tail == "early")
        );
        assert_eq!(r["stdout"], "earlylate");
        assert_eq!(r["stderr"], "error");
        assert_eq!(r["passed"], false);
        assert_eq!(updates.last().unwrap().exit_code, Some(7));
        assert!(!updates.last().unwrap().running);
    }
    #[test]
    fn command_timeout_and_user_stop_are_distinct() {
        let d = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let r = execute_with_progress(
            d.path(),
            "shell",
            &json!({"command":"printf ready; sleep 10; touch should-not-exist"}),
            &cancel,
            &mut |p| {
                if p.stdout_tail == "ready" {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
        )
        .unwrap();
        assert_eq!(r["stopped"], true);
        assert_eq!(r["timed_out"], false);
        assert!(!d.path().join("should-not-exist").exists());
        let r = execute(
            d.path(),
            "shell",
            &json!({"command":"sleep 10; touch should-not-exist","timeout_secs":1}),
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(r["timed_out"], true);
        assert_eq!(r["stopped"], false);
        assert_eq!(r["passed"], false);
        assert!(!d.path().join("should-not-exist").exists());
        assert!(shell_timeout(&json!({"timeout_secs":0})).is_err());
        assert!(shell_timeout(&json!({"timeout_secs":121})).is_err());
        assert!(shell_timeout(&json!({"timeout_secs":"30"})).is_err());
    }
    #[test]
    fn large_command_output_retains_its_beginning_and_failure_tail() {
        let mut capture = Capture::default();
        capture.push(b"START");
        for _ in 0..100 {
            capture.push(&[b'x'; 4096]);
        }
        capture.push(b"FINAL FAILURE");
        assert_eq!(capture.bytes.len(), 32_000);
        assert!(capture.text().starts_with("START"));
        assert!(capture.text().contains("output truncated"));
        assert!(capture.text().ends_with("FINAL FAILURE"));
        assert!(capture.tail().ends_with("FINAL FAILURE"));
    }
    #[test]
    fn serialized_json_expectations_preserve_types() {
        let d = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        execute(
            d.path(),
            "write_file",
            &json!({"path":"result.json","content":"{\"ready\":true}"}),
            &cancel,
        )
        .unwrap();
        let correct = execute(
            d.path(),
            "check_file",
            &json!({"path":"result.json","kind":"json_equals","expected":"{\"ready\":true}"}),
            &cancel,
        )
        .unwrap();
        assert_eq!(correct["passed"], true);
        assert_eq!(correct["expected"], json!({"ready":true}));
        let string_value = serde_json::to_string("{\"ready\":true}").unwrap();
        assert_eq!(
            execute(
                d.path(),
                "check_file",
                &json!({"path":"result.json","kind":"json_equals","expected":string_value}),
                &cancel
            )
            .unwrap()["passed"],
            false
        );
    }
}
