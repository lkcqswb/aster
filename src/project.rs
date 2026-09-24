//! Bounded project navigation with explicit continuation and ignore rules.
use anyhow::{Context, Result, bail};
use globset::{GlobBuilder, GlobMatcher};
use ignore::WalkBuilder;
use regex::{Regex, RegexBuilder};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub const MAX_SOURCE: usize = 2_000_000;
const PAGE_BYTES: usize = 16_000;
const MAX_ENTRIES: usize = 20_000;
const MAX_SCAN_BYTES: usize = 64_000_000;

fn escaped_size(c: char) -> usize {
    match c {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{0008}' | '\u{000c}' => 2,
        c if c < '\u{0020}' => 6,
        c => c.len_utf8(),
    }
}

fn number(args: &Value, key: &str, default: usize, min: usize, max: usize) -> Result<usize> {
    let value = args
        .get(key)
        .map(|v| {
            v.as_u64()
                .with_context(|| format!("{key} must be an integer"))
        })
        .transpose()?
        .unwrap_or(default as u64);
    if value < min as u64 || value > max as u64 {
        bail!("{key} must be between {min} and {max}");
    }
    Ok(value as usize)
}
fn boolean(args: &Value, key: &str, default: bool) -> Result<bool> {
    args.get(key)
        .map(|v| {
            v.as_bool()
                .with_context(|| format!("{key} must be true or false"))
        })
        .transpose()
        .map(|v| v.unwrap_or(default))
}
fn directory(root: &Path, args: &Value) -> Result<PathBuf> {
    let name = args
        .get("path")
        .map(|v| v.as_str().context("path must be text"))
        .transpose()?
        .unwrap_or(".");
    let path = if name == "." {
        root.to_owned()
    } else {
        crate::tools::path(root, name)?
    };
    if !path.is_dir() {
        bail!("Search path must be a project directory");
    }
    Ok(path)
}
fn glob(args: &Value) -> Result<Option<GlobMatcher>> {
    let Some(pattern) = args.get("glob") else {
        return Ok(None);
    };
    let pattern = pattern.as_str().context("glob must be text")?;
    if pattern.is_empty() || pattern.len() > 500 {
        bail!("glob needs 1–500 bytes");
    }
    Ok(Some(
        GlobBuilder::new(pattern)
            .literal_separator(true)
            .case_insensitive(!boolean(args, "case_sensitive", true)?)
            .build()?
            .compile_matcher(),
    ))
}
fn walker(root: &Path, directory: &Path) -> ignore::Walk {
    let scope = directory.to_owned();
    WalkBuilder::new(root)
        .hidden(false)
        .parents(false)
        .git_global(false)
        .require_git(false)
        .follow_links(false)
        .sort_by_file_path(|a, b| a.cmp(b))
        .filter_entry(move |entry| {
            entry.depth() == 0
                || ((entry.path().starts_with(&scope) || scope.starts_with(entry.path()))
                    && !crate::tools::sensitive(&entry.file_name().to_string_lossy())
                    && !entry.path_is_symlink())
        })
        .build()
}
fn stopped(cancel: &Arc<AtomicBool>) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!("Stopped");
    }
    Ok(())
}

pub fn text(root: &Path, name: &str) -> Result<String> {
    let path = crate::tools::path(root, name)?;
    let metadata = fs::metadata(&path)?;
    if !metadata.is_file() || metadata.len() > MAX_SOURCE as u64 {
        bail!("Read requires a regular UTF-8 file of at most 2 MB");
    }
    let file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_SOURCE as u64 {
        bail!("Read requires a regular UTF-8 file of at most 2 MB");
    }
    let mut bytes = Vec::new();
    file.take((MAX_SOURCE + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SOURCE {
        bail!("File grew beyond the 2 MB read limit");
    }
    if bytes.contains(&0) {
        bail!("Binary file is not supported");
    }
    String::from_utf8(bytes).context("File is not UTF-8 text")
}

pub fn read_page(root: &Path, args: &Value) -> Result<Value> {
    let name = args["path"].as_str().context("path must be text")?;
    let offset = number(args, "offset", 1, 1, MAX_SOURCE + 1)?;
    let column = number(args, "column", 1, 1, MAX_SOURCE + 1)?;
    let limit = number(args, "limit", 200, 1, 500)?;
    let source = text(root, name)?;
    let lines = source.lines().collect::<Vec<_>>();
    let mut content = String::new();
    let mut serialized_bytes = 0;
    let mut next = None;
    let mut returned = 0;
    for (index, line) in lines.iter().enumerate().skip(offset - 1).take(limit) {
        let start_column = if index == offset - 1 { column } else { 1 };
        let start = line.char_indices().nth(start_column - 1).map(|(i, _)| i);
        let start = match start {
            Some(start) => start,
            None if start_column == line.chars().count() + 1 => line.len(),
            None => bail!("column is beyond the selected line"),
        };
        let prefix = if start_column == 1 {
            format!("{}: ", index + 1)
        } else {
            format!("{} [column {start_column}]: ", index + 1)
        };
        let separator = usize::from(!content.is_empty());
        let remaining = PAGE_BYTES.saturating_sub(content.len() + prefix.len() + separator);
        let prefix_cost = prefix.chars().map(escaped_size).sum::<usize>() + separator * 2;
        let json_remaining = 24_000usize.saturating_sub(serialized_bytes + prefix_cost);
        if remaining < 4 || json_remaining < 6 {
            next = Some((index + 1, start_column));
            break;
        }
        let tail = &line[start..];
        let mut end = 0;
        let mut encoded = 0;
        for (byte, c) in tail.char_indices() {
            let cost = escaped_size(c);
            if byte + c.len_utf8() > remaining || encoded + cost > json_remaining {
                break;
            }
            encoded += cost;
            end = byte + c.len_utf8();
        }
        if separator > 0 {
            content.push('\n');
        }
        content.push_str(&prefix);
        content.push_str(&tail[..end]);
        serialized_bytes += prefix_cost + encoded;
        returned += 1;
        if end < tail.len() {
            next = Some((index + 1, start_column + tail[..end].chars().count()));
            break;
        }
        next = if index + 1 < lines.len() {
            Some((index + 2, 1))
        } else {
            None
        };
    }
    Ok(
        json!({"path":name,"content":content,"offset":offset,"column":column,"returned_lines":returned,"total_lines":lines.len(),"source_bytes":source.len(),"next_offset":next.map(|x|x.0),"next_column":next.map(|x|x.1)}),
    )
}

pub fn list(root: &Path, args: &Value, cancel: &Arc<AtomicBool>) -> Result<Value> {
    let directory = directory(root, args)?;
    let glob = glob(args)?;
    let offset = number(args, "offset", 0, 0, MAX_ENTRIES)?;
    let limit = number(args, "limit", 200, 1, 500)?;
    let started = Instant::now();
    let mut files = Vec::new();
    let mut matched = 0;
    let mut output_bytes = 0;
    let mut next = None;
    let mut incomplete = None;
    let mut errors = 0;
    for (visited, entry) in walker(root, &directory).enumerate() {
        stopped(cancel)?;
        if visited >= MAX_ENTRIES || started.elapsed() > Duration::from_secs(5) {
            incomplete = Some("Scan limit reached; narrow path or glob");
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                errors += 1;
                continue;
            }
        };
        if entry.error().is_some() {
            errors += 1;
        }
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let relative = entry.path().strip_prefix(root)?;
        if glob.as_ref().is_some_and(|g| !g.is_match(relative)) {
            continue;
        }
        let name = relative.to_string_lossy().into_owned();
        if matched < offset {
            matched += 1;
            continue;
        }
        let encoded = serde_json::to_string(&name)?.len() + 1;
        if files.len() == limit || output_bytes + encoded > PAGE_BYTES {
            next = Some(matched);
            break;
        }
        output_bytes += encoded;
        matched += 1;
        files.push(name);
    }
    Ok(
        json!({"files":files,"offset":offset,"next_offset":next,"truncated":next.is_some()||incomplete.is_some()||errors>0,"incomplete_reason":incomplete,"scan_errors":errors,"ignores_respected":true}),
    )
}

fn matcher(args: &Value) -> Result<Regex> {
    let query = args["query"].as_str().context("query must be text")?;
    if query.is_empty() || query.len() > 500 {
        bail!("Search needs 1–500 bytes");
    }
    let pattern = if boolean(args, "regex", false)? {
        query.to_owned()
    } else {
        regex::escape(query)
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(!boolean(args, "case_sensitive", true)?)
        .size_limit(1_000_000)
        .dfa_size_limit(1_000_000)
        .build()
        .context("Invalid or excessively complex search pattern")
}

pub fn search(root: &Path, args: &Value, cancel: &Arc<AtomicBool>) -> Result<Value> {
    let directory = directory(root, args)?;
    let glob = glob(args)?;
    let matcher = matcher(args)?;
    let offset = number(args, "offset", 0, 0, MAX_ENTRIES)?;
    let limit = number(args, "limit", 50, 1, 100)?;
    let context = number(args, "context", 0, 0, 3)?;
    let mut matches = Vec::new();
    let mut matched = 0;
    let mut scanned_files = 0;
    let mut scanned_bytes = 0;
    let mut skipped_files = 0;
    let mut scan_errors = 0;
    let mut output_bytes = 0;
    let mut next = None;
    let mut incomplete = None;
    let started = Instant::now();
    'files: for (visited, entry) in walker(root, &directory).enumerate() {
        stopped(cancel)?;
        if visited >= MAX_ENTRIES || started.elapsed() > Duration::from_secs(5) {
            incomplete = Some("Scan limit reached; narrow path or glob");
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                scan_errors += 1;
                continue;
            }
        };
        if entry.error().is_some() {
            scan_errors += 1;
        }
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let relative = entry.path().strip_prefix(root)?;
        if glob.as_ref().is_some_and(|g| !g.is_match(relative)) {
            continue;
        }
        let name = relative.to_string_lossy();
        let bytes = entry.metadata().map(|m| m.len() as usize).unwrap_or(0);
        if bytes > MAX_SOURCE {
            skipped_files += 1;
            continue;
        }
        if scanned_bytes + bytes > MAX_SCAN_BYTES {
            incomplete = Some("64 MB scan limit reached; narrow path or glob");
            break;
        }
        let source = match text(root, &name) {
            Ok(source) => source,
            Err(_) => {
                skipped_files += 1;
                continue;
            }
        };
        scanned_files += 1;
        scanned_bytes += source.len();
        let lines = source.lines().collect::<Vec<_>>();
        for (index, line) in lines.iter().enumerate() {
            if index % 256 == 0 {
                stopped(cancel)?;
                if started.elapsed() > Duration::from_secs(5) {
                    incomplete = Some("Search time limit reached; narrow path or query");
                    break 'files;
                }
            }
            let Some(found) = matcher.find(line) else {
                continue;
            };
            if matched < offset {
                matched += 1;
                continue;
            }
            let start_column = line[..found.start()].chars().count() + 1;
            let mut start = found.start().saturating_sub(100);
            while !line.is_char_boundary(start) {
                start += 1;
            }
            let excerpt = crate::tools::clip(&line[start..], 700);
            let before = lines[index.saturating_sub(context)..index].iter().enumerate().map(|(i, line)| json!({"line":index.saturating_sub(context)+i+1,"text":crate::tools::clip(line,300)})).collect::<Vec<_>>();
            let after = lines[index + 1..(index + 1 + context).min(lines.len())]
                .iter()
                .enumerate()
                .map(|(i, line)| json!({"line":index+i+2,"text":crate::tools::clip(line,300)}))
                .collect::<Vec<_>>();
            let row = json!({"path":name,"line":index+1,"column":start_column,"text":excerpt,"excerpt_column":line[..start].chars().count()+1,"before":before,"after":after});
            let size = serde_json::to_vec(&row)?.len();
            if matches.len() == limit || output_bytes + size > 24_000 {
                next = Some(matched);
                break 'files;
            }
            output_bytes += size;
            matches.push(row);
            matched += 1;
        }
    }
    Ok(
        json!({"matches":matches,"offset":offset,"next_offset":next,"truncated":next.is_some()||incomplete.is_some()||scan_errors>0,"incomplete_reason":incomplete,"scanned_files":scanned_files,"scanned_bytes":scanned_bytes,"skipped_files":skipped_files,"scan_errors":scan_errors,"ignores_respected":true}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cancel() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    #[test]
    fn listings_respect_root_and_nested_ignores_and_have_exact_pages() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src/deep/a/b/c/d/e/f/g")).unwrap();
        fs::write(root.join(".gitignore"), "ignored.rs\n").unwrap();
        fs::write(root.join("src/.gitignore"), "nested.rs\n").unwrap();
        for path in [
            "src/a.rs",
            "src/b.rs",
            "src/c.rs",
            "src/ignored.rs",
            "src/nested.rs",
            "src/.env",
            "src/key.pem",
        ] {
            fs::write(root.join(path), "fixture").unwrap();
        }
        fs::write(root.join("src/deep/a/b/c/d/e/f/g/last.rs"), "fixture").unwrap();
        let first = list(
            root,
            &json!({"path":"src","glob":"src/**/*.rs","limit":2}),
            &cancel(),
        )
        .unwrap();
        assert_eq!(first["files"], json!(["src/a.rs", "src/b.rs"]));
        assert_eq!(first["next_offset"], 2);
        let second = list(
            root,
            &json!({"path":"src","glob":"src/**/*.rs","limit":2,"offset":2}),
            &cancel(),
        )
        .unwrap();
        assert_eq!(
            second["files"],
            json!(["src/c.rs", "src/deep/a/b/c/d/e/f/g/last.rs"])
        );
        assert!(second["next_offset"].is_null());
        assert_eq!(second["truncated"], false);
        assert!(list(root, &json!({"path":"../"}), &cancel()).is_err());
    }

    #[test]
    fn regex_context_case_and_matching_line_pagination_work_together() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("src/main.rs"),
            "before\nTASK_42\nbetween\ntask_99\nafter\n",
        )
        .unwrap();
        fs::write(dir.path().join("src/.env"), "TASK_00").unwrap();
        let args = json!({"query":"task_[0-9]+","regex":true,"case_sensitive":false,"glob":"**/*.rs","context":1,"limit":1});
        let first = search(dir.path(), &args, &cancel()).unwrap();
        assert_eq!(first["matches"][0]["line"], 2);
        assert_eq!(first["matches"][0]["before"][0]["text"], "before");
        assert_eq!(first["matches"][0]["after"][0]["text"], "between");
        assert_eq!(first["next_offset"], 1);
        let mut next = args.clone();
        next["offset"] = json!(1);
        let second = search(dir.path(), &next, &cancel()).unwrap();
        assert_eq!(second["matches"][0]["line"], 4);
        assert!(second["next_offset"].is_null());
        assert_eq!(second["truncated"], false);
        assert!(search(dir.path(), &json!({"query":"[","regex":true}), &cancel()).is_err());
        assert!(
            search(
                dir.path(),
                &json!({"query":"task","case_sensitive":"false"}),
                &cancel()
            )
            .is_err()
        );
        assert!(
            search(
                dir.path(),
                &json!({"query":"task"}),
                &Arc::new(AtomicBool::new(true))
            )
            .is_err()
        );
    }
    #[test]
    fn invalid_ignore_rules_are_reported_as_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "[z-a]\n").unwrap();
        fs::write(dir.path().join("source.txt"), "needle").unwrap();
        let result = search(dir.path(), &json!({"query":"needle"}), &cancel()).unwrap();
        assert!(result["scan_errors"].as_u64().unwrap() > 0);
        assert_eq!(result["truncated"], true);
        assert_eq!(result["matches"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn large_unicode_lines_continue_without_losing_characters() {
        let dir = tempfile::tempdir().unwrap();
        let long = "玉".repeat(60_000);
        fs::write(dir.path().join("large.txt"), format!("{long}\nlast\n")).unwrap();
        let mut args = json!({"path":"large.txt","limit":1});
        let mut reconstructed = String::new();
        loop {
            let page = read_page(dir.path(), &args).unwrap();
            let content = page["content"].as_str().unwrap();
            assert!(content.len() <= PAGE_BYTES);
            reconstructed.push_str(content.split_once(": ").unwrap().1);
            if page["next_offset"] == 2 {
                break;
            }
            assert_eq!(page["next_offset"], 1);
            assert!(page["next_column"].as_u64().unwrap() > args["column"].as_u64().unwrap_or(1));
            args["offset"] = page["next_offset"].clone();
            args["column"] = page["next_column"].clone();
        }
        assert_eq!(reconstructed, long);
        let last = read_page(dir.path(), &json!({"path":"large.txt","offset":2})).unwrap();
        assert_eq!(last["content"], "2: last");
        assert!(last["next_offset"].is_null());
        assert!(
            read_page(
                dir.path(),
                &json!({"path":"large.txt","offset":2,"column":99})
            )
            .is_err()
        );
    }
    #[test]
    fn escaped_text_pages_leave_room_for_valid_continuation_metadata() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("escaped.txt"), "\u{0001}".repeat(10_000)).unwrap();
        let page = read_page(dir.path(), &json!({"path":"escaped.txt"})).unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() < 25_000);
        assert_eq!(page["next_offset"], 1);
        assert!(page["next_column"].as_u64().unwrap() > 1);
    }

    #[test]
    fn search_excerpts_include_late_matches_and_report_skipped_sources() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("long.txt"),
            format!("{}needle\n", "玉".repeat(60_000)),
        )
        .unwrap();
        fs::write(dir.path().join("binary.bin"), b"\0needle").unwrap();
        fs::write(dir.path().join("oversize.txt"), vec![b'x'; MAX_SOURCE + 1]).unwrap();
        let result = search(dir.path(), &json!({"query":"needle"}), &cancel()).unwrap();
        assert_eq!(result["matches"].as_array().unwrap().len(), 1);
        assert!(
            result["matches"][0]["text"]
                .as_str()
                .unwrap()
                .contains("needle")
        );
        assert_eq!(result["matches"][0]["column"], 60_001);
        assert_eq!(result["skipped_files"], 2);
        assert!(text(dir.path(), "binary.bin").is_err());
        assert!(text(dir.path(), "oversize.txt").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_private_paths_cannot_be_enabled_by_globs() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".aster")).unwrap();
        fs::write(dir.path().join(".aster/private.txt"), "needle").unwrap();
        fs::write(dir.path().join(".env"), "needle").unwrap();
        fs::write(dir.path().join("PRIVATE.PEM"), "needle").unwrap();
        std::os::unix::fs::symlink(dir.path().join(".env"), dir.path().join("alias.txt")).unwrap();
        let result = search(
            dir.path(),
            &json!({"query":"needle","glob":"**/*"}),
            &cancel(),
        )
        .unwrap();
        assert!(result["matches"].as_array().unwrap().is_empty());
        assert!(text(dir.path(), "alias.txt").is_err());
        assert!(text(dir.path(), ".ENV").is_err());
        assert!(text(dir.path(), "PRIVATE.PEM").is_err());
        assert!(list(dir.path(), &json!({"path":".aster"}), &cancel()).is_err());
    }
}
