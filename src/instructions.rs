use anyhow::{Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug)]
pub struct Rule {
    pub path: PathBuf,
    pub text: String,
}
pub fn load(project: &Path) -> Result<Vec<Rule>> {
    let mut paths = vec![];
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(PathBuf::from(home).join(".config/aster/AGENTS.md"));
    }
    let mut parents = project.ancestors().collect::<Vec<_>>();
    parents.reverse();
    paths.extend(parents.into_iter().map(|p| p.join("AGENTS.md")));
    read_rules(paths)
}
pub fn scoped(project: &Path, file: &Path) -> Result<Vec<Rule>> {
    let parent = file.parent().unwrap_or(project);
    let mut dirs = vec![];
    for p in parent.ancestors() {
        if p == project {
            break;
        }
        if p.starts_with(project) {
            dirs.push(p.join("AGENTS.md"));
        }
    }
    dirs.reverse();
    read_rules(dirs)
}
fn read_rules(paths: Vec<PathBuf>) -> Result<Vec<Rule>> {
    let mut rules = vec![];
    let mut size = 0;
    for p in paths {
        if p.is_file() {
            let metadata = fs::metadata(&p)?;
            if metadata.len() > 32_000 {
                bail!("Instruction file exceeds 32 KB: {}", p.display())
            };
            let text = fs::read_to_string(&p)?;
            size += text.len();
            if size > 64_000 {
                bail!("AGENTS.md instructions exceed 64 KB")
            };
            rules.push(Rule { path: p, text });
        }
    }
    Ok(rules)
}
pub fn format(rules: &[Rule]) -> String {
    rules
        .iter()
        .map(|r| format!("\n--- Instructions: {} ---\n{}\n", r.path.display(), r.text))
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directory_scope_is_ordered() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("a/b")).unwrap();
        fs::write(d.path().join("a/AGENTS.md"), "outer").unwrap();
        fs::write(d.path().join("a/b/AGENTS.md"), "inner").unwrap();
        let r = scoped(d.path(), &d.path().join("a/b/x")).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].text, "outer");
        assert_eq!(r[1].text, "inner");
    }
}
