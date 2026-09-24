//! Prepare exactly what the user reviews, then refuse to overwrite intervening edits.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    fs,
    hash::Hasher,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

pub struct PreparedEdit {
    pub relative: String,
    pub before: Option<String>,
    pub after: String,
    pub diff: String,
    action: Action,
}
/// The reviewed operation. Moves and deletions keep a fingerprint of what was previewed.
enum Action {
    Write,
    Move {
        to: String,
        source: Snapshot,
        destination: Option<Snapshot>,
    },
    Delete {
        file: Snapshot,
    },
}
#[derive(Debug, PartialEq)]
struct Snapshot {
    bytes: u64,
    modified: Option<SystemTime>,
    digest: Option<u64>,
}
/// Files larger than this are fingerprinted by size and modification time only.
const DIGEST_LIMIT: u64 = 64_000_000;
const FRAGMENT: usize = 128_000;
/// Fingerprint a regular file without following a final symlink; `None` if it is missing.
fn snapshot(path: &Path) -> Result<Option<Snapshot>> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !meta.is_file() {
        bail!("Only a regular file can be moved or deleted");
    }
    let digest = if meta.len() <= DIGEST_LIMIT {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let mut file = fs::File::open(path)?.take(DIGEST_LIMIT + 1);
        let mut buffer = vec![0; 64 * 1024];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.write(&buffer[..n]);
        }
        Some(hasher.finish())
    } else {
        None
    };
    Ok(Some(Snapshot {
        bytes: meta.len(),
        modified: meta.modified().ok(),
        digest,
    }))
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
    match tool {
        "move_file" => return prepare_move(root, args),
        "delete_file" => return prepare_delete(root, args),
        _ => {}
    }
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
        "multi_edit" => apply_edits(
            before
                .as_deref()
                .context("Cannot edit a missing file; use write_file to create it")?,
            args["edits"].as_array().context("edits must be an array")?,
        )?,
        _ => bail!("Not a file editing tool"),
    };
    if after.len() > crate::project::MAX_SOURCE {
        bail!("Edited file exceeds 2 MB");
    }
    let diff = if tool == "multi_edit" {
        hunks(relative, before.as_deref().unwrap_or(""), &after)
    } else {
        diff(relative, before.as_deref().unwrap_or(""), &after)
    };
    Ok(PreparedEdit {
        relative: relative.into(),
        before,
        after,
        diff,
        action: Action::Write,
    })
}
/// Apply every edit in memory, in order. Any failure names its edit and changes nothing.
fn apply_edits(source: &str, edits: &[Value]) -> Result<String> {
    if edits.is_empty() || edits.len() > 32 {
        bail!("edits needs 1–32 items");
    }
    let total = edits.len();
    let mut current = source.to_string();
    for (index, edit) in edits.iter().enumerate() {
        let label = format!("edits[{index}] (edit {} of {total})", index + 1);
        let old = edit["old_text"]
            .as_str()
            .with_context(|| format!("{label}: old_text must be text"))?;
        let new = edit["new_text"]
            .as_str()
            .with_context(|| format!("{label}: new_text must be text"))?;
        let all = flag(edit, "replace_all").map_err(|e| anyhow::anyhow!("{label}: {e}"))?;
        if old.len() > FRAGMENT || new.len() > FRAGMENT {
            bail!("{label}: use edit fragments of at most 128 KB. Nothing was changed.");
        }
        if old.is_empty() {
            bail!("{label}: old_text must be nonempty. Nothing was changed.");
        }
        let count = current.matches(old).count();
        if count == 0 {
            bail!(
                "{label}: old_text was not found{}. Read the file and match its current text exactly. Nothing was changed.",
                if index > 0 {
                    " after applying the earlier edits"
                } else {
                    ""
                }
            );
        }
        if count > 1 && !all {
            bail!(
                "{label}: expected exactly one match for old_text, found {count}. Include more context, or set replace_all to replace every occurrence. Nothing was changed."
            );
        }
        let replaced = if all { count } else { 1 };
        let size = current.len() - replaced * old.len() + replaced * new.len();
        if size > crate::project::MAX_SOURCE {
            bail!("{label}: the edited file would exceed 2 MB. Nothing was changed.");
        }
        current = if all {
            current.replace(old, new)
        } else {
            current.replacen(old, new, 1)
        };
    }
    Ok(current)
}
fn flag(args: &Value, key: &str) -> Result<bool> {
    args.get(key)
        .map(|v| {
            v.as_bool()
                .with_context(|| format!("{key} must be true or false"))
        })
        .transpose()
        .map(|v| v.unwrap_or(false))
}
fn rename(from: &Path, to: &Path) -> Result<()> {
    fs::rename(from, to).map_err(|e| {
        if e.kind() == std::io::ErrorKind::CrossesDevices {
            anyhow::anyhow!("Cannot move a file across filesystems; nothing was moved")
        } else {
            e.into()
        }
    })
}
fn prepare_move(root: &Path, args: &Value) -> Result<PreparedEdit> {
    let from = args["from"].as_str().context("from must be a string")?;
    let to = args["to"].as_str().context("to must be a string")?;
    let overwrite = flag(args, "overwrite")?;
    let source_path = crate::tools::path(root, from)?;
    let target_path = crate::tools::path(root, to)?;
    if source_path == target_path {
        bail!("from and to name the same file; nothing to move");
    }
    let source_meta = match fs::symlink_metadata(&source_path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!("{from} does not exist; nothing was moved")
        }
        Err(e) => return Err(e.into()),
    };
    if source_meta.is_dir() {
        bail!("move_file moves one regular file; {from} is a directory. Nothing was moved.");
    }
    let source = snapshot(&source_path)?.context("Source file disappeared")?;
    let parent = target_path
        .parent()
        .context("Destination has no parent directory")?;
    let shown = |p: &Path| p.strip_prefix(root).unwrap_or(p).display().to_string();
    let mut creates = None;
    for ancestor in parent.ancestors() {
        if ancestor == root || !ancestor.starts_with(root) {
            break;
        }
        match fs::symlink_metadata(ancestor) {
            Ok(meta) if meta.is_dir() => break,
            Ok(_) => bail!("{} is not a directory; nothing was moved", shown(ancestor)),
            Err(_) => {
                creates.get_or_insert_with(|| shown(parent));
            }
        }
    }
    let destination = match fs::symlink_metadata(&target_path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
        Ok(meta) if meta.is_dir() => {
            bail!("{to} is a directory; name the destination file itself. Nothing was moved.")
        }
        Ok(_) if !overwrite => {
            bail!("{to} already exists. Set overwrite to true to replace it; nothing was moved.")
        }
        Ok(meta) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if (meta.dev(), meta.ino()) == (source_meta.dev(), source_meta.ino()) {
                    bail!("{from} and {to} are the same file; nothing to move");
                }
            }
            #[cfg(not(unix))]
            let _ = meta;
            snapshot(&target_path)?
        }
    };
    let mut diff = format!(
        "move_file: {from} → {to}\nrename from {from}\nrename to {to}\nSize: {} bytes · content unchanged\n",
        source.bytes
    );
    if let Some(directory) = creates {
        diff += &format!("Creates directory: {directory}\n");
    }
    if let Some(replaced) = &destination {
        diff += &format!(
            "\nReplaces existing {to} ({} bytes), which will be lost:\n{}",
            replaced.bytes,
            removal(to, &target_path, replaced.bytes)?
        );
    }
    Ok(PreparedEdit {
        relative: from.into(),
        before: None,
        after: String::new(),
        diff,
        action: Action::Move {
            to: to.into(),
            source,
            destination,
        },
    })
}
fn prepare_delete(root: &Path, args: &Value) -> Result<PreparedEdit> {
    let relative = args["path"].as_str().context("path must be a string")?;
    let path = crate::tools::path(root, relative)?;
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_dir() => bail!(
            "delete_file removes one regular file; {relative} is a directory. Nothing was deleted."
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!("{relative} does not exist; nothing was deleted")
        }
        _ => {}
    }
    let file = snapshot(&path)?.context("File disappeared")?;
    let diff = format!(
        "delete_file: {relative} · {} bytes\n{}",
        file.bytes,
        removal(relative, &path, file.bytes)?
    );
    Ok(PreparedEdit {
        relative: relative.into(),
        before: None,
        after: String::new(),
        diff,
        action: Action::Delete { file },
    })
}
/// A short removal diff: the first lines of a text file, or a note for binary content.
fn removal(name: &str, path: &Path, bytes: u64) -> Result<String> {
    const HEAD_BYTES: u64 = 4_000;
    const HEAD_LINES: usize = 20;
    let mut head = Vec::new();
    fs::File::open(path)?
        .take(HEAD_BYTES)
        .read_to_end(&mut head)?;
    let complete = head.len() as u64 >= bytes;
    let text = match std::str::from_utf8(&head) {
        Ok(text) => Some(text),
        Err(e) if !complete && e.error_len().is_none() => {
            std::str::from_utf8(&head[..e.valid_up_to()]).ok()
        }
        Err(_) => None,
    };
    let header = format!("--- a/{name}\n+++ /dev/null\n");
    let Some(text) = text.filter(|t| !t.contains('\0')) else {
        return Ok(format!(
            "{header}Binary content · {bytes} bytes · not shown\n"
        ));
    };
    if bytes == 0 {
        return Ok(format!("{header}(empty file)\n"));
    }
    let lines = text.split_inclusive('\n').collect::<Vec<_>>();
    let shown = lines.len().min(HEAD_LINES);
    let mut out = if complete {
        format!("{header}@@ -1,{} +0,0 @@\n", lines.len())
    } else {
        format!("{header}@@ first {shown} lines of {bytes} bytes @@\n")
    };
    for line in &lines[..shown] {
        out.push('-');
        out.push_str(&crate::tools::clip(
            line.trim_end_matches(['\n', '\r']),
            240,
        ));
        out.push('\n');
    }
    if !complete || shown < lines.len() {
        out += "… more content not shown\n";
    }
    Ok(out)
}
impl PreparedEdit {
    pub fn commit(&self, root: &Path) -> Result<Value> {
        match &self.action {
            Action::Write => self.write(root),
            Action::Move {
                to,
                source,
                destination,
            } => self.relocate(root, to, source, destination.as_ref()),
            Action::Delete { file } => {
                let path = crate::tools::path(root, &self.relative)?;
                if snapshot(&path)?.as_ref() != Some(file) {
                    bail!(
                        "{} changed or disappeared after this deletion was prepared. Nothing was deleted; prepare it again.",
                        self.relative
                    );
                }
                fs::remove_file(&path)?;
                Ok(json!({"path":self.relative,"deleted":true,"bytes":file.bytes,"diff":self.diff}))
            }
        }
    }
    fn relocate(
        &self,
        root: &Path,
        to: &str,
        source: &Snapshot,
        destination: Option<&Snapshot>,
    ) -> Result<Value> {
        let from = &self.relative;
        let from_path = crate::tools::path(root, from)?;
        if snapshot(&from_path)?.as_ref() != Some(source) {
            bail!(
                "{from} changed or disappeared after this move was prepared. Nothing was moved; prepare it again."
            );
        }
        let target = crate::tools::path(root, to)?;
        let appeared = if destination.is_some() {
            "changed"
        } else {
            "appeared"
        };
        if fs::symlink_metadata(&target).is_ok_and(|m| !m.is_file())
            || snapshot(&target)?.as_ref() != destination
        {
            bail!(
                "{to} {appeared} after this move was prepared. Nothing was moved or overwritten."
            );
        }
        fs::create_dir_all(
            target
                .parent()
                .context("Destination has no parent directory")?,
        )?;
        // Recheck the destination path after creating directories.
        let target = crate::tools::path(root, to)?;
        if destination.is_some() {
            rename(&from_path, &target)?;
        } else {
            // A hard link never replaces an existing file, closing the check-then-move gap.
            match fs::hard_link(&from_path, &target) {
                Ok(()) => {
                    if let Err(e) = fs::remove_file(&from_path) {
                        let _ = fs::remove_file(&target);
                        return Err(e).context("Could not remove the original; nothing was moved");
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    bail!("{to} appeared before commit. Nothing was moved or overwritten.")
                }
                Err(_) => {
                    if fs::symlink_metadata(&target).is_ok() {
                        bail!("{to} appeared before commit. Nothing was moved or overwritten.");
                    }
                    rename(&from_path, &target)?;
                }
            }
        }
        Ok(
            json!({"from":from,"to":to,"moved":true,"bytes":source.bytes,"overwrote":destination.is_some(),"diff":self.diff}),
        )
    }
    fn write(&self, root: &Path) -> Result<Value> {
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
/// A unified diff with a separate hunk per distant change, so every edit in a large file
/// stays visible in a bounded preview. Falls back to `diff` when the change is too large.
pub fn hunks(name: &str, before: &str, after: &str) -> String {
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
    let (a, b) = (
        &old[prefix..old.len() - suffix],
        &new[prefix..new.len() - suffix],
    );
    let budget = (40_000_000 / (a.len() + b.len() + 1)).clamp(16, 1_000);
    let Some(middle) = shortest_edit(a, b, budget) else {
        return diff(name, before, after);
    };
    let ops = old[..prefix]
        .iter()
        .map(|t| (' ', *t))
        .chain(middle)
        .chain(old[old.len() - suffix..].iter().map(|t| (' ', *t)))
        .collect::<Vec<_>>();
    // Line numbers before each operation.
    let mut numbers = Vec::with_capacity(ops.len() + 1);
    let (mut o, mut n) = (0, 0);
    for (mark, _) in &ops {
        numbers.push((o, n));
        o += usize::from(*mark != '+');
        n += usize::from(*mark != '-');
    }
    numbers.push((o, n));
    let changes = ops
        .iter()
        .enumerate()
        .filter(|(_, (mark, _))| *mark != ' ')
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    let mut out = format!("--- a/{name}\n+++ b/{name}\n");
    let mut index = 0;
    while index < changes.len() {
        let first = changes[index];
        let mut last = first;
        while index + 1 < changes.len() && changes[index + 1] - last <= 7 {
            index += 1;
            last = changes[index];
        }
        index += 1;
        let start = first.saturating_sub(3);
        let end = (last + 4).min(ops.len());
        let (old_start, new_start) = numbers[start];
        let old_count = numbers[end].0 - old_start;
        let new_count = numbers[end].1 - new_start;
        let position = |start: usize, count: usize| if count == 0 { start } else { start + 1 };
        out += &format!(
            "@@ -{},{old_count} +{},{new_count} @@\n",
            position(old_start, old_count),
            position(new_start, new_count)
        );
        for (mark, text) in &ops[start..end] {
            out.push(*mark);
            out.push_str(text);
            if !text.ends_with('\n') {
                out.push_str("\n\\ No newline at end of file\n");
            }
        }
    }
    out
}
/// Myers' shortest edit script over lines, abandoned beyond `limit` differences.
fn shortest_edit<'a>(a: &[&'a str], b: &[&'a str], limit: usize) -> Option<Vec<(char, &'a str)>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let limit = limit.min(a.len() + b.len()) as isize;
    let offset = limit + 1;
    let at = |k: isize| (k + offset) as usize;
    let mut v = vec![0i32; (2 * limit + 3) as usize];
    let mut trace = Vec::new();
    for d in 0..=limit {
        trace.push(v.clone());
        for k in (-d..=d).step_by(2) {
            let mut x = if k == -d || (k != d && v[at(k - 1)] < v[at(k + 1)]) {
                v[at(k + 1)] as isize
            } else {
                v[at(k - 1)] as isize + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[at(k)] = x as i32;
            if x >= n && y >= m {
                let mut script = Vec::new();
                let (mut x, mut y) = (n, m);
                for d in (0..=d).rev() {
                    let v = &trace[d as usize];
                    let k = x - y;
                    let previous = if k == -d || (k != d && v[at(k - 1)] < v[at(k + 1)]) {
                        k + 1
                    } else {
                        k - 1
                    };
                    let px = v[at(previous)] as isize;
                    let py = px - previous;
                    while x > px && y > py {
                        script.push((' ', a[x as usize - 1]));
                        x -= 1;
                        y -= 1;
                    }
                    if d > 0 {
                        if x == px {
                            script.push(('+', b[y as usize - 1]));
                        } else {
                            script.push(('-', a[x as usize - 1]));
                        }
                    }
                    (x, y) = (px, py);
                }
                script.reverse();
                return Some(script);
            }
        }
    }
    None
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

    fn multi(dir: &Path, path: &str, edits: Value) -> Result<PreparedEdit> {
        prepare(dir, "multi_edit", &json!({"path":path,"edits":edits}))
    }
    /// Apply a rendered unified diff to `before`, proving each hunk describes the real change.
    fn apply(before: &str, diff: &str) -> String {
        let old: Vec<_> = before.split_inclusive('\n').collect();
        let mut out = String::new();
        let mut used = 0;
        let mut lines = diff.lines().skip(2).peekable();
        while let Some(header) = lines.next() {
            let start: usize = header[4..]
                .split([',', ' '])
                .next()
                .unwrap()
                .parse()
                .unwrap();
            let count: usize = header
                .split(' ')
                .nth(1)
                .unwrap()
                .split(',')
                .nth(1)
                .unwrap()
                .parse()
                .unwrap();
            let start = if count == 0 { start } else { start - 1 };
            for line in &old[used..start] {
                out.push_str(line);
            }
            used = start;
            let mut last: Option<char> = None;
            while let Some(line) = lines.peek().filter(|l| !l.starts_with("@@")) {
                let line = *line;
                lines.next();
                if line == "\\ No newline at end of file" {
                    if last != Some('-') {
                        out.pop();
                    }
                    continue;
                }
                let (mark, text) = line.split_at(1);
                last = mark.chars().next();
                if mark != "+" {
                    used += 1;
                }
                if mark != "-" {
                    out.push_str(text);
                    out.push('\n');
                }
            }
        }
        for line in &old[used..] {
            out.push_str(line);
        }
        out
    }
    #[test]
    fn multi_edit_applies_in_order_and_shows_every_distant_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.rs");
        let original = (1..=400).map(|i| format!("line {i}\n")).collect::<String>();
        fs::write(&path, &original).unwrap();
        let edit = multi(
            dir.path(),
            "big.rs",
            json!([
                {"old_text":"line 5\n","new_text":"line five\n"},
                {"old_text":"line five","new_text":"line FIVE"},
                {"old_text":"line 3","new_text":"LINE 3","replace_all":true},
                {"old_text":"LINE 399\n","new_text":""},
                {"old_text":"line 200\n","new_text":"line two hundred\n"}
            ]),
        )
        .unwrap();
        let expected = original
            .replace("line 5\n", "line FIVE\n")
            .replace("line 3", "LINE 3")
            .replace("LINE 399\n", "")
            .replace("line 200\n", "line two hundred\n");
        assert_eq!(edit.after, expected);
        assert_eq!(apply(&original, &edit.diff), expected);
        assert!(edit.diff.contains("-line 5\n+line FIVE\n"));
        assert!(edit.diff.contains("-line 200\n+line two hundred\n"));
        assert!(edit.diff.contains("-line 399\n"));
        assert!(edit.diff.contains("+LINE 300\n"));
        assert_eq!(edit.diff.matches("@@ -").count(), 4);
        assert!(edit.diff.len() < 12_000, "{}", edit.diff.len());
        edit.commit(dir.path()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    }
    #[test]
    fn hunk_diffs_round_trip_insertions_deletions_and_missing_newlines() {
        let mut seed = 7u64;
        let mut next = |n: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        for case in 0..60 {
            let before = (0..next(40))
                .map(|_| format!("v{}\n", next(6)))
                .collect::<String>();
            let mut after = String::new();
            for line in before.lines() {
                if next(5) != 0 {
                    after += &format!("{line}\n");
                }
                if next(6) == 0 {
                    after += &format!("new{}\n", next(4));
                }
            }
            if case % 3 == 0 {
                after.pop();
            }
            if before == after {
                continue;
            }
            let rendered = hunks("f", &before, &after);
            assert_eq!(apply(&before, &rendered), after, "case {case}\n{rendered}");
        }
        assert_eq!(hunks("f", "same\n", "same\n"), "f: no changes");
        assert_eq!(apply("a\nb", &hunks("f", "a\nb", "a\nc\n")), "a\nc\n");
    }
    #[test]
    fn multi_edit_failures_name_the_edit_and_change_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        fs::write(&path, "alpha beta beta\n").unwrap();
        for (edits, message) in [
            (
                json!([{"old_text":"alpha","new_text":"A"},{"old_text":"beta","new_text":"B"}]),
                "edits[1] (edit 2 of 2): expected exactly one match for old_text, found 2",
            ),
            (
                json!([{"old_text":"alpha","new_text":"A"},{"old_text":"alpha","new_text":"B"}]),
                "edits[1] (edit 2 of 2): old_text was not found after applying the earlier edits",
            ),
            (
                json!([{"old_text":"gamma","new_text":"G"}]),
                "edits[0] (edit 1 of 1): old_text was not found.",
            ),
            (
                json!([{"old_text":"","new_text":"G"}]),
                "edits[0] (edit 1 of 1): old_text must be nonempty",
            ),
            (
                json!([{"old_text":"alpha","new_text":"x".repeat(FRAGMENT + 1)}]),
                "edits[0] (edit 1 of 1): use edit fragments of at most 128 KB",
            ),
            (
                json!([{"old_text":"a","new_text":"x".repeat(FRAGMENT),"replace_all":true},{"old_text":"x".repeat(1000),"new_text":"y".repeat(FRAGMENT),"replace_all":true}]),
                "edits[1] (edit 2 of 2): the edited file would exceed 2 MB",
            ),
            (json!([]), "edits needs 1–32 items"),
        ] {
            let error = multi(dir.path(), "a.rs", edits).err().unwrap().to_string();
            assert!(error.contains(message), "{error}");
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "alpha beta beta\n");
        let error = multi(
            dir.path(),
            "missing.rs",
            json!([{"old_text":"a","new_text":"b"}]),
        )
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains("missing file"), "{error}");
        let replaced = multi(
            dir.path(),
            "a.rs",
            json!([{"old_text":"beta","new_text":"B","replace_all":true}]),
        )
        .unwrap();
        assert_eq!(replaced.after, "alpha B B\n");
        fs::write(&path, "alpha beta beta\nuser line\n").unwrap();
        let error = replaced.commit(dir.path()).unwrap_err().to_string();
        assert!(error.contains("File changed after"), "{error}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "alpha beta beta\nuser line\n"
        );
    }
    #[test]
    fn moves_preview_create_directories_and_refuse_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.txt"), "first\nsecond\n").unwrap();
        fs::write(root.join("b.txt"), "keep me\n").unwrap();
        let mv = |args: Value| prepare(root, "move_file", &args);
        let error = mv(json!({"from":"a.txt","to":"b.txt"}))
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("already exists") && error.contains("overwrite"),
            "{error}"
        );
        let error = mv(json!({"from":"a.txt","to":"./a.txt"}))
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("same file"), "{error}");
        let error = mv(json!({"from":"none.txt","to":"c.txt"}))
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("does not exist"), "{error}");
        fs::create_dir(root.join("folder")).unwrap();
        let error = mv(json!({"from":"folder","to":"moved"}))
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("is a directory"), "{error}");
        let error = mv(json!({"from":"a.txt","to":"folder"}))
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("is a directory"), "{error}");
        let error = mv(json!({"from":"a.txt","to":"b.txt/inner.txt"}))
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("not a directory"), "{error}");
        for (from, to) in [
            ("a.txt", ".git/a.txt"),
            (".env", "env.txt"),
            ("a.txt", "../outside.txt"),
            ("a.txt", "/tmp/outside.txt"),
            ("a.txt", "nested/.ssh/key"),
        ] {
            fs::write(root.join(".env"), "SECRET=1").unwrap();
            assert!(mv(json!({"from":from,"to":to})).is_err(), "{from} → {to}");
        }
        let prepared = mv(json!({"from":"a.txt","to":"src/deep/a.txt"})).unwrap();
        assert!(
            prepared
                .diff
                .contains("rename from a.txt\nrename to src/deep/a.txt")
        );
        assert!(prepared.diff.contains("Creates directory: src/deep\n"));
        assert!(root.join("a.txt").exists() && !root.join("src").exists());
        let result = prepared.commit(root).unwrap();
        assert_eq!(result["moved"], true);
        assert_eq!(result["overwrote"], false);
        assert!(!root.join("a.txt").exists());
        assert_eq!(
            fs::read_to_string(root.join("src/deep/a.txt")).unwrap(),
            "first\nsecond\n"
        );
        let replace = mv(json!({"from":"src/deep/a.txt","to":"b.txt","overwrite":true})).unwrap();
        assert!(replace.diff.contains("Replaces existing b.txt (8 bytes)"));
        assert!(replace.diff.contains("-keep me"));
        replace.commit(root).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("b.txt")).unwrap(),
            "first\nsecond\n"
        );
    }
    #[test]
    fn moves_refuse_a_changed_source_or_a_destination_that_appeared() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.txt"), "original").unwrap();
        let prepared = prepare(root, "move_file", &json!({"from":"a.txt","to":"b.txt"})).unwrap();
        fs::write(root.join("a.txt"), "edited!!").unwrap();
        let error = prepared.commit(root).unwrap_err().to_string();
        assert!(error.contains("a.txt changed"), "{error}");
        assert!(!root.join("b.txt").exists());
        let prepared = prepare(root, "move_file", &json!({"from":"a.txt","to":"b.txt"})).unwrap();
        fs::write(root.join("b.txt"), "someone else's").unwrap();
        let error = prepared.commit(root).unwrap_err().to_string();
        assert!(error.contains("b.txt appeared"), "{error}");
        assert_eq!(
            fs::read_to_string(root.join("b.txt")).unwrap(),
            "someone else's"
        );
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "edited!!");
        let overwrite = prepare(
            root,
            "move_file",
            &json!({"from":"a.txt","to":"b.txt","overwrite":true}),
        )
        .unwrap();
        fs::write(root.join("b.txt"), "changed again").unwrap();
        let error = overwrite.commit(root).unwrap_err().to_string();
        assert!(error.contains("b.txt changed"), "{error}");
        assert_eq!(
            fs::read_to_string(root.join("b.txt")).unwrap(),
            "changed again"
        );
        let prepared = prepare(root, "move_file", &json!({"from":"a.txt","to":"c.txt"})).unwrap();
        fs::remove_file(root.join("a.txt")).unwrap();
        assert!(prepared.commit(root).is_err());
        assert!(!root.join("c.txt").exists());
    }
    #[test]
    fn deletes_preview_the_content_and_refuse_stale_or_directory_targets() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let text = (1..=30).map(|i| format!("row {i}\n")).collect::<String>();
        fs::write(root.join("notes.txt"), &text).unwrap();
        let prepared = prepare(root, "delete_file", &json!({"path":"notes.txt"})).unwrap();
        assert!(prepared.diff.starts_with(&format!(
            "delete_file: notes.txt · {} bytes\n--- a/notes.txt\n+++ /dev/null\n@@ -1,30 +0,0 @@\n-row 1\n",
            text.len()
        )));
        assert!(prepared.diff.contains("-row 20\n… more content not shown"));
        assert!(!prepared.diff.contains("row 21"));
        fs::write(root.join("notes.txt"), text.replace("row 30", "row 31")).unwrap();
        let error = prepared.commit(root).unwrap_err().to_string();
        assert!(error.contains("changed or disappeared"), "{error}");
        assert!(root.join("notes.txt").exists());
        let fresh = prepare(root, "delete_file", &json!({"path":"notes.txt"})).unwrap();
        assert_eq!(fresh.commit(root).unwrap()["deleted"], true);
        assert!(!root.join("notes.txt").exists());
        assert!(fresh.commit(root).is_err());
        fs::write(root.join("image.png"), [0x89, b'P', 0, 0, 1]).unwrap();
        let binary = prepare(root, "delete_file", &json!({"path":"image.png"})).unwrap();
        assert!(binary.diff.contains("Binary content · 5 bytes · not shown"));
        fs::create_dir(root.join("folder")).unwrap();
        let error = prepare(root, "delete_file", &json!({"path":"folder"}))
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("is a directory"), "{error}");
        assert!(root.join("folder").is_dir());
        for path in [
            "missing.txt",
            ".env",
            "../x",
            "/etc/hosts",
            ".git/config",
            "id.pem",
        ] {
            assert!(
                prepare(root, "delete_file", &json!({"path":path})).is_err(),
                "{path}"
            );
        }
    }
    #[cfg(unix)]
    #[test]
    fn moves_and_deletes_refuse_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(outside.path().join("target.txt"), "outside").unwrap();
        fs::write(root.join("real.txt"), "inside").unwrap();
        std::os::unix::fs::symlink(outside.path().join("target.txt"), root.join("link.txt"))
            .unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("out")).unwrap();
        for (tool, args) in [
            ("delete_file", json!({"path":"link.txt"})),
            ("move_file", json!({"from":"link.txt","to":"moved.txt"})),
            (
                "move_file",
                json!({"from":"real.txt","to":"out/escaped.txt"}),
            ),
            (
                "move_file",
                json!({"from":"real.txt","to":"link.txt","overwrite":true}),
            ),
            (
                "multi_edit",
                json!({"path":"link.txt","edits":[{"old_text":"o","new_text":"x"}]}),
            ),
        ] {
            let error = prepare(root, tool, &args).err().unwrap().to_string();
            assert!(error.contains("Symlink"), "{tool}: {error}");
        }
        let prepared = prepare(
            root,
            "move_file",
            &json!({"from":"real.txt","to":"sub/real.txt"}),
        )
        .unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("sub")).unwrap();
        assert!(prepared.commit(root).is_err());
        assert!(!outside.path().join("real.txt").exists());
        assert_eq!(
            fs::read_to_string(outside.path().join("target.txt")).unwrap(),
            "outside"
        );
    }
}
