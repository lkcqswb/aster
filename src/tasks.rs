//! Explicit, local project commands. Discovery never executes a command.
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{collections::HashSet, fs, io::Read, path::Path};

fn timeout() -> u64 {
    120
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub command: String,
    #[serde(default = "timeout")]
    pub timeout_secs: u64,
    #[serde(skip)]
    pub source: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    tasks: Vec<Task>,
}
#[derive(Default)]
pub struct Catalog {
    pub tasks: Vec<Task>,
    pub notes: Vec<String>,
}
fn name_valid(name: &str) -> bool {
    name.bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphanumeric())
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':'))
}
fn read(root: &Path, relative: &str) -> Result<Option<String>> {
    let path = root.join(relative);
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if meta.file_type().is_symlink()
        || !meta.is_file()
        || !path.canonicalize()?.starts_with(root.canonicalize()?)
    {
        bail!("Task source must be a regular file inside the project: {relative}");
    }
    if meta.len() > 64_000 {
        bail!("Task source exceeds 64 KB: {relative}");
    }
    let mut text = String::new();
    fs::File::open(path)?
        .take(64_001)
        .read_to_string(&mut text)?;
    if text.len() > 64_000 {
        bail!("Task source grew beyond 64 KB: {relative}");
    }
    Ok(Some(text))
}
pub fn discover(root: &Path) -> Result<Catalog> {
    if let Some(text) = read(root, ".aster/tasks.json")? {
        let file: File = serde_json::from_str(&text).context("Invalid .aster/tasks.json")?;
        if file.tasks.len() > 32 {
            bail!("Use at most 32 project tasks");
        }
        let mut names = HashSet::new();
        let mut tasks = file.tasks;
        for task in &mut tasks {
            if !name_valid(&task.name) || !names.insert(task.name.clone()) {
                bail!(
                    "Task names must be unique, start with an ASCII letter or number, and use at most 64 letters, numbers, hyphens, underscores or colons"
                );
            }
            if task.description.len() > 240
                || task.command.trim().is_empty()
                || task.command.len() > 4000
                || task.command.contains('\0')
                || !(1..=120).contains(&task.timeout_secs)
            {
                bail!(
                    "Task {} needs a command of 1–4000 bytes, description up to 240 bytes and timeout of 1–120 seconds",
                    task.name
                );
            }
            task.source = ".aster/tasks.json".into();
        }
        return Ok(Catalog {
            tasks,
            notes: vec!["Explicit project tasks replace automatic discovery.".into()],
        });
    }
    let mut catalog = Catalog::default();
    if read(root, "Cargo.toml")?.is_some() {
        for command in ["test", "check", "build"] {
            catalog.tasks.push(Task {
                name: format!("cargo:{command}"),
                description: format!("Run cargo {command}"),
                command: format!("cargo {command}"),
                timeout_secs: 120,
                source: "Cargo.toml".into(),
            });
        }
    }
    if let Some(text) = read(root, "package.json")? {
        let package: serde_json::Value =
            serde_json::from_str(&text).context("Invalid package.json during task discovery")?;
        let manager = if root.join("pnpm-lock.yaml").is_file() {
            "pnpm"
        } else if root.join("yarn.lock").is_file() {
            "yarn"
        } else if root.join("bun.lock").is_file() || root.join("bun.lockb").is_file() {
            "bun"
        } else {
            "npm"
        };
        for (name, script) in package["scripts"].as_object().into_iter().flatten() {
            if !name_valid(name) || name.len() + manager.len() + 1 > 64 || !script.is_string() {
                catalog.notes.push(format!(
                    "Skipped script with unsupported name or value: {}",
                    crate::tools::clip(name, 64)
                ));
                continue;
            }
            if catalog.tasks.len() == 32 {
                catalog.notes.push("Additional scripts omitted. Define .aster/tasks.json to choose tasks explicitly.".into());
                break;
            }
            catalog.tasks.push(Task {
                name: format!("{manager}:{name}"),
                description: crate::tools::clip(script.as_str().unwrap_or(""), 220),
                command: format!("{manager} run {name}"),
                timeout_secs: 120,
                source: "package.json scripts".into(),
            });
        }
    }
    if catalog.tasks.is_empty() {
        catalog.notes.push(
            "No project tasks found. Define .aster/tasks.json, or use /run for a literal command."
                .into(),
        );
    }
    Ok(catalog)
}
pub fn resolve(root: &Path, name: &str) -> Result<Task> {
    discover(root)?
        .tasks
        .into_iter()
        .find(|task| task.name == name)
        .with_context(|| {
            format!("No project task named {name}. Use /tasks to inspect available tasks.")
        })
}
impl Task {
    pub fn details(&self) -> String {
        format!(
            "{}\n{}\n\nSource: {}\nTimeout: {} seconds\n\n$ {}\n\nEnter runs this local command with your current permissions.\nNo model request is used. Plan mode blocks execution.\nEsc returns to the task list.",
            self.name, self.description, self.source, self.timeout_secs, self.command
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_tasks_replace_discovery_and_reject_duplicates_and_unknown_fields() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join(".aster")).unwrap();
        fs::write(d.path().join("Cargo.toml"), "[package]").unwrap();
        let path = d.path().join(".aster/tasks.json");
        fs::write(
            &path,
            r#"{"tasks":[{"name":"test","command":"printf proof","timeout_secs":2}]}"#,
        )
        .unwrap();
        let catalog = discover(d.path()).unwrap();
        assert_eq!(catalog.tasks.len(), 1);
        assert_eq!(resolve(d.path(), "test").unwrap().timeout_secs, 2);
        fs::write(
            &path,
            r#"{"tasks":[{"name":"x","command":"one"},{"name":"x","command":"two"}]}"#,
        )
        .unwrap();
        assert!(discover(d.path()).is_err());
        fs::write(
            &path,
            r#"{"tasks":[{"name":"x","command":"one","unknown":true}]}"#,
        )
        .unwrap();
        assert!(discover(d.path()).is_err());
    }
    #[test]
    fn discovered_commands_use_the_project_manager_and_never_execute() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("Cargo.toml"), "[package]").unwrap();
        fs::write(d.path().join("package.json"),r#"{"scripts":{"test":"touch should-not-exist","test:unit":"vitest run","bad;name":"false"}}"#).unwrap();
        fs::write(d.path().join("pnpm-lock.yaml"), "").unwrap();
        let tasks = discover(d.path()).unwrap();
        assert_eq!(tasks.tasks.len(), 5);
        assert_eq!(
            resolve(d.path(), "pnpm:test:unit").unwrap().command,
            "pnpm run test:unit"
        );
        assert!(!d.path().join("should-not-exist").exists());
        assert_eq!(tasks.notes.len(), 1);
    }
    #[cfg(unix)]
    #[test]
    fn task_sources_cannot_follow_symlinks_outside_the_project() {
        let d = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join(".aster")).unwrap();
        let source = outside.path().join("tasks.json");
        fs::write(&source, "{\"tasks\":[]}").unwrap();
        std::os::unix::fs::symlink(source, d.path().join(".aster/tasks.json")).unwrap();
        assert!(discover(d.path()).is_err());
    }
}
