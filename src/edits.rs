//! Prepare exactly what the user reviews, then refuse to overwrite intervening edits.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub struct PreparedEdit {
    pub relative: String,
    pub before: Option<String>,
    pub after: String,
    pub diff: String,
}
fn contents(path: &Path) -> Result<Option<String>> {
    match fs::metadata(path) {
        Ok(meta) => {
            if !meta.is_file() || meta.len() > crate::project::MAX_SOURCE as u64 {
                bail!("Edit requires a UTF-8 file of at most 2 MB");
            }
            let mut content = String::new();
            fs::File::open(path)?
                .take((crate::project::MAX_SOURCE + 1) as u64)
                .read_to_string(&mut content)?;
            if content.len() > crate::project::MAX_SOURCE || content.contains('\0') {
                bail!("Edit requires a UTF-8 text file of at most 2 MB");
            }
            Ok(Some(content))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
pub fn prepare(root: &Path, tool: &str, args: &Value) -> Result<PreparedEdit> {
    let relative = args["path"].as_str().context("path must be a string")?;
    let path = crate::tools::path(root, relative)?;
    let before = contents(&path)?;
    let after = match tool {
        "write_file" => {
            if before.as_ref().is_some_and(|text| text.len() > 128_000) {
                bail!("Use a focused edit_file change for an existing file larger than 128 KB");
            }
            let content = args["content"].as_str().context("content must be text")?;
            if content.len() > 128_000 {
                bail!("Write exceeds 128 KB");
            }
            content.to_string()
        }
        "edit_file" => {
            let old = args["old_text"].as_str().context("old_text must be text")?;
            let new = args["new_text"].as_str().context("new_text must be text")?;
            if old.len() > 128_000 || new.len() > 128_000 {
                bail!("Use edit fragments of at most 128 KB");
            }
            if old.is_empty() {
                bail!("old_text must be nonempty; use write_file to create a file");
            }
            let current = before.as_deref().context("Cannot edit a missing file")?;
            let count = current.match_indices(old).count();
            if count != 1 {
                bail!(
                    "Expected exactly one match for old_text, found {count}. Read the file and include more context."
                );
            }
            current.replacen(old, new, 1)
        }
        _ => bail!("Not a file editing tool"),
    };
    if after.len() > crate::project::MAX_SOURCE {
        bail!("Edited file exceeds 2 MB");
    }
    let diff = diff(relative, before.as_deref().unwrap_or(""), &after);
    Ok(PreparedEdit {
        relative: relative.into(),
        before,
        after,
        diff,
    })
}
impl PreparedEdit {
    pub fn commit(&self, root: &Path) -> Result<Value> {
        let path = crate::tools::path(root, &self.relative)?;
        if contents(&path)? != self.before {
            bail!(
                "File changed after this edit was prepared. Read it again and prepare a fresh edit; nothing was overwritten."
            );
        }
        if self.before.as_deref() == Some(&self.after) {
            return Ok(
                json!({"path":self.relative,"written":false,"unchanged":true,"diff":self.diff}),
            );
        }
        let parent = path.parent().context("File has no parent directory")?;
        fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        if let Ok(meta) = fs::metadata(&path) {
            temporary.as_file().set_permissions(meta.permissions())?;
        }
        temporary.write_all(self.after.as_bytes())?;
        temporary.as_file().sync_all()?;
        // Recheck both the path and bytes after preparing the temporary file.
        let verified: PathBuf = crate::tools::path(root, &self.relative)?;
        if contents(&verified)? != self.before {
            bail!("File changed before commit; nothing was overwritten");
        }
        temporary.persist(verified)?;
        Ok(json!({"path":self.relative,"bytes":self.after.len(),"written":true,"diff":self.diff}))
    }
}
pub fn diff(name: &str, before: &str, after: &str) -> String {
    if before == after {
        return format!("{name}: no changes");
    }
    let old: Vec<_> = before.split_inclusive('\n').collect();
    let new: Vec<_> = after.split_inclusive('\n').collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let start = prefix.saturating_sub(3);
    let old_end = (old.len() - suffix + 3).min(old.len());
    let new_end = (new.len() - suffix + 3).min(new.len());
    let mut out = format!(
        "--- a/{name}\n+++ b/{name}\n@@ -{},{} +{},{} @@\n",
        start + 1,
        old_end - start,
        start + 1,
        new_end - start
    );
    let mut append = |mark: char, text: &str| {
        out.push(mark);
        out.push_str(text);
        if !text.ends_with('\n') {
            out.push_str("\n\\ No newline at end of file\n");
        }
    };
    for text in &old[start..prefix] {
        append(' ', text);
    }
    for text in &old[prefix..old.len() - suffix] {
        append('-', text);
    }
    for text in &new[prefix..new.len() - suffix] {
        append('+', text);
    }
    for text in &old[old.len() - suffix..old_end] {
        append(' ', text);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_source_allows_only_a_focused_edit_and_rejects_a_stale_commit() {
        let dir = tempfile::tempdir().unwrap();
        let original = format!(
            "{}target = bad\n{}",
            "// unchanged\n".repeat(10_000),
            "// tail\n".repeat(10_000)
        );
        let path = dir.path().join("large.rs");
        fs::write(&path, &original).unwrap();
        assert!(
            prepare(
                dir.path(),
                "write_file",
                &json!({"path":"large.rs","content":"replacement"})
            )
            .is_err()
        );
        let edit = prepare(
            dir.path(),
            "edit_file",
            &json!({"path":"large.rs","old_text":"target = bad","new_text":"target = good"}),
        )
        .unwrap();
        assert!(edit.diff.len() < 300);
        edit.commit(dir.path()).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            original.replace("target = bad", "target = good")
        );
        assert!(edit.commit(dir.path()).is_err());
    }
    #[test]
    fn exact_edit_preserves_surrounding_content_and_rejects_stale_approval() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.rs");
        fs::write(&path, "// keep\nlet count = 1;\n// tail\n").unwrap();
        let args = json!({"path":"main.rs","old_text":"count = 1","new_text":"count = 2"});
        let edit = prepare(dir.path(), "edit_file", &args).unwrap();
        assert!(edit.diff.contains("-let count = 1;"));
        assert!(edit.diff.contains("+let count = 2;"));
        fs::write(&path, "user changed this\n").unwrap();
        assert!(edit.commit(dir.path()).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "user changed this\n");
        fs::write(&path, "// keep\nlet count = 1;\n// tail\n").unwrap();
        edit.commit(dir.path()).unwrap();
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "// keep\nlet count = 2;\n// tail\n"
        );
    }
    #[test]
    fn ambiguous_edits_and_deleted_files_do_not_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a");
        fs::write(&path, "same same").unwrap();
        assert!(
            prepare(
                dir.path(),
                "edit_file",
                &json!({"path":"a","old_text":"same","new_text":"x"})
            )
            .is_err()
        );
        let edit = prepare(
            dir.path(),
            "write_file",
            &json!({"path":"a","content":"replacement"}),
        )
        .unwrap();
        fs::remove_file(path).unwrap();
        assert!(edit.commit(dir.path()).is_err());
    }
}
