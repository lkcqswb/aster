use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub fn schemas() -> Value {
    json!([
     {"name":"list_files","description":"List project files, excluding credentials and generated/private directories.","input_schema":{"type":"object","properties":{},"additionalProperties":false}},
     {"name":"read_file","description":"Read a UTF-8 project file, up to 128 KB. Nested AGENTS.md guidance is provided before file access.","input_schema":{"type":"object","properties":{"path":{"type":"string","description":"Relative to the project root, such as src/main.rs. Never an absolute path."}},"required":["path"],"additionalProperties":false}},
     {"name":"search","description":"Find a literal string in project text files. Bounded to 100 matching lines.","input_schema":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false}},
     {"name":"write_file","description":"Create or replace a UTF-8 project file. Requires user permission in ask mode. Respect AGENTS.md.","input_schema":{"type":"object","properties":{"path":{"type":"string","description":"Relative to the project root, such as src/main.rs. Never an absolute path."},"content":{"type":"string"}},"required":["path","content"],"additionalProperties":false}},
     {"name":"shell","description":"Run a shell command in the project, with a 30-second timeout and bounded output. Requires explicit permission in ask mode. It is NOT a filesystem sandbox.","input_schema":{"type":"object","properties":{"command":{"type":"string"}},"required":["command"],"additionalProperties":false}},
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
    matches!(name, "write_file" | "shell")
}
pub fn preview(root: &Path, name: &str, a: &Value) -> String {
    if name == "shell" {
        return format!(
            "$ {}\n\nRuns in {}. Shell access is not sandboxed.",
            a["command"].as_str().unwrap_or(""),
            root.display()
        );
    }
    let filename = a["path"].as_str().unwrap_or("");
    let before = path(root, filename)
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_default();
    format!(
        "{}\n\nBEFORE\n{}\n\nAFTER\n{}",
        filename,
        clip(&before, 5000),
        clip(a["content"].as_str().unwrap_or(""), 5000)
    )
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
pub fn execute(root: &Path, name: &str, a: &Value, cancel: &Arc<AtomicBool>) -> Result<Value> {
    if cancel.load(Ordering::Relaxed) {
        bail!("Stopped")
    }
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
    match name {
        "list_files" => Ok(json!({"files":files(root)?,"limit":800})),
        "read_file" => {
            Ok(json!({"path":string(a,"path")?,"content":read(root,string(a,"path")?)?}))
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
        "write_file" => {
            let content = string(a, "content")?;
            if content.len() > 128_000 {
                bail!("Write exceeds 128 KB")
            };
            let p = path(root, string(a, "path")?)?;
            fs::create_dir_all(p.parent().unwrap())?;
            if cancel.load(Ordering::Relaxed) {
                bail!("Stopped")
            };
            crate::session::private_write(&p, content.as_bytes())?;
            Ok(json!({"path":string(a,"path")?,"bytes":content.len(),"written":true}))
        }
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
        "shell" => shell(root, string(a, "command")?, cancel),
        _ => bail!("Unknown tool"),
    }
}
fn shell(root: &Path, command: &str, cancel: &Arc<AtomicBool>) -> Result<Value> {
    if command.len() > 8000 {
        bail!("Command is too long")
    }
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Do not hand model-generated commands the provider or host service credentials.
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
    let mut child = cmd.spawn()?;
    let drain = |mut stream: Box<dyn Read + Send>| {
        thread::spawn(move || {
            let mut kept = vec![];
            let mut buf = [0u8; 4096];
            while let Ok(n) = stream.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let take = n.min(32_000usize.saturating_sub(kept.len()));
                kept.extend_from_slice(&buf[..take]);
            }
            String::from_utf8_lossy(&kept).into_owned()
        })
    };
    let out = drain(Box::new(child.stdout.take().unwrap()));
    let err = drain(Box::new(child.stderr.take().unwrap()));
    let start = Instant::now();
    let mut stopped = false;
    let status = loop {
        if cancel.load(Ordering::Relaxed) || start.elapsed() > Duration::from_secs(30) {
            stopped = true;
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            break child.wait()?;
        }
        if let Some(s) = child.try_wait()? {
            break s;
        }
        thread::sleep(Duration::from_millis(50));
    };
    // Reap background children in this tool's own process group before joining pipe readers.
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    Ok(
        json!({"exit_code":status.code(),"stdout":out.join().unwrap_or_default(),"stderr":err.join().unwrap_or_default(),"stopped":stopped,"passed":status.success()&&!stopped}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
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
