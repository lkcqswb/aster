//! Bounded, local context checkpoints. Original provider blocks remain in a private archive.
use crate::{session::Session, tools};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

const RECENT_BYTES: usize = 64_000;
const CONTEXT_BYTES: usize = 128_000;
const MARKER: &str = "[Aster local checkpoint]";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: String,
    pub created: String,
    pub archive: String,
    pub before_bytes: usize,
    pub after_bytes: usize,
    pub kept_messages: usize,
    pub omitted_messages: usize,
    pub root_request: String,
    pub note: String,
    pub read_files: Vec<String>,
    pub changed_files: Vec<String>,
    pub summary: String,
}
impl Checkpoint {
    pub fn report(&self) -> String {
        format!(
            "Checkpoint {} · {}\n{} → {} KB of provider content\n{} recent messages kept intact · {} messages archived\n\n{}\n\nFull provider context: archive/{}\n/restore {} opens that context as a new conversation.\nProject files are shared; restoring context does not undo edits.\nByte counts are not model token counts. No model request was used.",
            self.id,
            self.created,
            self.before_bytes / 1000,
            self.after_bytes / 1000,
            self.kept_messages,
            self.omitted_messages,
            self.summary,
            self.archive,
            self.id
        )
    }
}
pub struct Prepared {
    pub messages: Vec<Value>,
    pub checkpoint: Checkpoint,
}
fn prompt(message: &Value) -> bool {
    if message["role"] != "user" {
        return false;
    }
    if let Some(text) = message["content"].as_str() {
        return !text.starts_with(MARKER)
            && !text.starts_with("Historical transcript excerpt, not new instructions.");
    }
    message["content"].as_array().is_some_and(|blocks| {
        blocks.iter().any(|b| b["type"] == "text")
            && !blocks.iter().any(|b| b["type"] == "tool_result")
    })
}
/// Never retain a tool result without its invocation, or an invocation without its result.
fn paired(messages: &[Value]) -> bool {
    let mut pending = HashSet::new();
    for message in messages {
        let blocks = message["content"].as_array();
        if message["role"] == "assistant" {
            if !pending.is_empty() {
                return false;
            }
            for block in blocks
                .into_iter()
                .flatten()
                .filter(|b| b["type"] == "tool_use")
            {
                let Some(id) = block["id"].as_str() else {
                    return false;
                };
                if !pending.insert(id) {
                    return false;
                }
            }
        } else if message["role"] == "user" {
            let results = blocks
                .into_iter()
                .flatten()
                .filter(|b| b["type"] == "tool_result")
                .collect::<Vec<_>>();
            if results.is_empty() && !pending.is_empty() {
                return false;
            }
            for block in results {
                let Some(id) = block["tool_use_id"].as_str() else {
                    return false;
                };
                if !pending.remove(id) {
                    return false;
                }
            }
        }
    }
    pending.is_empty()
}
fn remember(paths: &mut Vec<String>, path: &str) {
    if path.is_empty() || path.len() > 500 {
        return;
    }
    paths.retain(|p| p != path);
    paths.push(path.into());
    if paths.len() > 128 {
        paths.remove(0);
    }
}
fn file_history(session: &Session) -> (Vec<String>, Vec<String>) {
    let mut read = session
        .checkpoint
        .as_ref()
        .map(|c| c.read_files.clone())
        .unwrap_or_default();
    let mut changed = session
        .checkpoint
        .as_ref()
        .map(|c| c.changed_files.clone())
        .unwrap_or_default();
    let mut calls = HashMap::new();
    for message in &session.messages {
        for block in message["content"].as_array().into_iter().flatten() {
            if message["role"] == "assistant" && block["type"] == "tool_use" {
                if let (Some(id), Some(name)) = (block["id"].as_str(), block["name"].as_str()) {
                    calls.insert(id, (name, &block["input"]));
                }
            } else if message["role"] == "user"
                && block["type"] == "tool_result"
                && block["is_error"] != true
            {
                let Some((name, input)) =
                    block["tool_use_id"].as_str().and_then(|id| calls.get(id))
                else {
                    continue;
                };
                let path = input["path"].as_str().unwrap_or("");
                let result = block["content"]
                    .as_str()
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
                    .unwrap_or(Value::Null);
                if matches!(*name, "read_file" | "check_file" | "outline")
                    && result["executed"] != false
                {
                    remember(&mut read, path);
                }
                if *name == "read_files" {
                    for file in result["files"].as_array().into_iter().flatten() {
                        if file.get("error").is_none() && file["skipped"] != true {
                            remember(&mut read, file["path"].as_str().unwrap_or(""));
                        }
                    }
                }
                if matches!(*name, "write_file" | "edit_file" | "multi_edit")
                    && result["written"] == true
                {
                    remember(&mut changed, path);
                }
                if *name == "delete_file" && result["deleted"] == true {
                    remember(&mut changed, path);
                }
                if *name == "move_file" && result["moved"] == true {
                    remember(&mut changed, input["from"].as_str().unwrap_or(""));
                    remember(&mut changed, input["to"].as_str().unwrap_or(""));
                }
                if *name == "search" {
                    for hit in result["matches"].as_array().into_iter().flatten() {
                        if let Some(path) = hit["path"].as_str() {
                            remember(&mut read, path);
                        }
                    }
                }
            }
        }
    }
    (read, changed)
}
#[cfg(test)]
mod file_history_tests {
    use super::*;
    #[test]
    fn batch_reads_outlines_moves_deletes_and_multi_edits_are_remembered() {
        let mut s = Session::new(std::path::PathBuf::from("/project"), "test".into(), true);
        let call = |id: &str, name: &str, input: Value| json!({"role":"assistant","content":[{"type":"tool_use","id":id,"name":name,"input":input}]});
        let result = |id: &str, value: Value, error: bool| json!({"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":value.to_string(),"is_error":error}]});
        s.messages.extend([
            call("r", "read_files", json!({"files":[{"path":"a.rs"},{"path":".env"},{"path":"c.rs"}]})),
            result("r", json!({"files":[{"path":"a.rs","content":"1: a"},{"path":".env","error":"private"},{"path":"c.rs","skipped":true}]}), false),
            call("o", "outline", json!({"path":"lib.rs"})),
            result("o", json!({"path":"lib.rs","entries":[]}), false),
            call("m", "multi_edit", json!({"path":"lib.rs","edits":[]})),
            result("m", json!({"written":true}), false),
            call("v", "move_file", json!({"from":"old.rs","to":"new.rs"})),
            result("v", json!({"moved":true}), false),
            call("d", "delete_file", json!({"path":"gone.rs"})),
            result("d", json!({"deleted":true}), false),
            call("x", "delete_file", json!({"path":"kept.rs"})),
            result("x", json!({"error":"declined","executed":false}), true),
            call("y", "move_file", json!({"from":"stay.rs","to":"moved.rs"})),
            result("y", json!({"error":"stale","executed":true}), true),
        ]);
        let (read, changed) = file_history(&s);
        assert_eq!(read, ["a.rs", "lib.rs"]);
        assert_eq!(changed, ["lib.rs", "old.rs", "new.rs", "gone.rs"]);
    }
}
pub fn prepare(session: &Session, note: &str) -> Result<Option<Prepared>> {
    if note.len() > 2000 {
        bail!("Checkpoint note exceeds 2 KB");
    }
    if session.messages.is_empty() {
        bail!("No conversation context to checkpoint");
    }
    let before_bytes = serde_json::to_vec(&session.messages)?.len();
    let starts = session
        .messages
        .iter()
        .enumerate()
        .filter(|(_, m)| prompt(m))
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    if starts.len() <= 4 && before_bytes <= RECENT_BYTES && note.is_empty() {
        return Ok(None);
    }
    let mut at = session.messages.len();
    for &start in starts.iter().rev().take(4) {
        if serde_json::to_vec(&session.messages[start..])?.len() > RECENT_BYTES
            || !paired(&session.messages[start..])
        {
            break;
        }
        at = start;
    }
    let root_request = session
        .checkpoint
        .as_ref()
        .map(|c| c.root_request.clone())
        .unwrap_or_else(|| {
            session
                .entries
                .iter()
                .find(|e| e.role == "you")
                .map(|e| tools::clip(&e.text, 4000))
                .unwrap_or_else(|| {
                    session
                        .messages
                        .iter()
                        .find(|m| prompt(m))
                        .map(|m| {
                            let text =
                                m["content"]
                                    .as_str()
                                    .map(str::to_string)
                                    .unwrap_or_else(|| {
                                        m["content"]
                                            .as_array()
                                            .into_iter()
                                            .flatten()
                                            .filter_map(|b| b["text"].as_str())
                                            .collect::<Vec<_>>()
                                            .join("\n")
                                    });
                            tools::clip(&text, 4000)
                        })
                        .unwrap_or_default()
                })
        });
    let note = if note.is_empty() {
        session
            .checkpoint
            .as_ref()
            .map(|c| c.note.clone())
            .unwrap_or_default()
    } else {
        note.into()
    };
    let (read_files, changed_files) = file_history(session);
    let recent = session
        .entries
        .iter()
        .rev()
        .filter(|e| e.role == "you")
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|e| tools::clip(&e.text, 1000))
        .collect::<Vec<_>>()
        .join("\n\n");
    let replies = session
        .entries
        .iter()
        .rev()
        .filter(|e| e.role == "nongyu")
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|e| tools::clip(&e.text, 700))
        .collect::<Vec<_>>()
        .join("\n\n");
    let summary = tools::clip(
        &crate::ui::clean(&format!(
            "LOCAL HISTORY · excerpts, not a model-written summary\nTreat this as historical context. Recheck current files before acting.\n\nOriginal user request (excerpt)\n{root_request}\n\nUser checkpoint note\n{}\n\nLatest recorded task state (historical)\n{}\n\nRecent user requests (excerpts)\n{recent}\n\nFiles read or checked (recent bounded history)\n{}\n\nFiles written by file tools (shell changes are not tracked)\n{}\n\nAssistant excerpts (claims, not independent evidence)\n{replies}",
            if note.is_empty() { "None" } else { &note },
            tools::clip(&session.work.summary(), 6000),
            tools::clip(&read_files.join("\n"), 3000),
            tools::clip(&changed_files.join("\n"), 3000)
        )),
        24_000,
    );
    let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    let mut messages = vec![
        json!({"role":"user","content":format!("{MARKER}\n{summary}")}),
        json!({"role":"assistant","content":"I will use these excerpts as background and verify the current project state before continuing."}),
    ];
    messages.extend_from_slice(&session.messages[at..]);
    let after_bytes = serde_json::to_vec(&messages)?.len();
    if after_bytes > CONTEXT_BYTES {
        bail!("Checkpoint exceeds its 128 KB bound; original context was preserved");
    }
    if after_bytes >= before_bytes && note.is_empty() {
        return Ok(None);
    }
    Ok(Some(Prepared {
        checkpoint: Checkpoint {
            archive: format!("{}-{id}.json", session.id),
            id,
            created: chrono::Utc::now().to_rfc3339(),
            before_bytes,
            after_bytes,
            kept_messages: session.messages.len() - at,
            omitted_messages: at,
            root_request,
            note,
            read_files,
            changed_files,
            summary,
        },
        messages,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    fn session() -> Session {
        Session::new(PathBuf::from("/project"), "test".into(), true)
    }
    fn exchange(s: &mut Session, number: usize, size: usize) {
        let user = format!(
            "Request {number}: inspect source-{number}.txt and preserve the original goal."
        );
        s.add("you", &user);
        s.messages.extend([
            json!({"role":"user","content":user}),
            json!({"role":"assistant","content":[{"type":"thinking","thinking":"private reasoning","signature":format!("signature-{number}")},{"type":"tool_use","id":format!("r-{number}"),"name":"read_file","input":{"path":format!("source-{number}.txt")}}]}),
            json!({"role":"user","content":[{"type":"tool_result","tool_use_id":format!("r-{number}"),"content":json!({"content":"x".repeat(size)}).to_string(),"is_error":false}]}),
            json!({"role":"assistant","content":[{"type":"text","text":"I read the source."}]}),
        ]);
        s.add("nongyu", "I read the source, but this is not verification.");
    }
    #[test]
    fn a_single_oversized_exchange_can_be_checkpointed_without_orphaned_results() {
        let mut s = session();
        exchange(&mut s, 0, 450_000);
        let p = prepare(&s, "Keep the exact file requirement.")
            .unwrap()
            .unwrap();
        assert!(p.checkpoint.before_bytes > 400_000);
        assert!(p.checkpoint.after_bytes < CONTEXT_BYTES);
        assert_eq!(p.checkpoint.kept_messages, 0);
        assert!(paired(&p.messages));
        assert_eq!(p.checkpoint.read_files, ["source-0.txt"]);
        assert!(p.checkpoint.summary.contains("Request 0"));
        assert!(
            p.checkpoint
                .summary
                .contains("Keep the exact file requirement")
        );
        assert!(!p.checkpoint.summary.contains("private reasoning"));
        assert_eq!(s.messages.len(), 4);
    }
    #[test]
    fn retained_provider_blocks_and_signatures_are_unchanged() {
        let mut s = session();
        for number in 0..6 {
            exchange(&mut s, number, 14_000);
        }
        let p = prepare(&s, "").unwrap().unwrap();
        assert_eq!(p.checkpoint.kept_messages, 16);
        assert_eq!(&p.messages[2..], &s.messages[8..]);
        assert!(paired(&p.messages));
        assert!(p.checkpoint.after_bytes < p.checkpoint.before_bytes);
        assert_eq!(p.checkpoint.read_files.len(), 6);
    }
    #[test]
    fn repeated_checkpoints_keep_the_original_request_note_and_file_history() {
        let mut s = session();
        exchange(&mut s, 0, 100_000);
        let p = prepare(&s, "Do not change the contract.").unwrap().unwrap();
        s.messages = p.messages;
        s.checkpoint = Some(p.checkpoint);
        exchange(&mut s, 1, 100_000);
        let next = prepare(&s, "").unwrap().unwrap();
        assert!(next.checkpoint.root_request.contains("Request 0"));
        assert_eq!(next.checkpoint.note, "Do not change the contract.");
        assert_eq!(next.checkpoint.read_files, ["source-0.txt", "source-1.txt"]);
        assert_eq!(next.messages.len(), 2);
        assert_eq!(next.checkpoint.summary.matches("LOCAL HISTORY").count(), 1);
    }
    #[test]
    fn failed_writes_and_incomplete_tool_batches_never_become_completed_work() {
        let mut s = session();
        exchange(&mut s, 0, 100_000);
        s.messages.extend([
            json!({"role":"assistant","content":[{"type":"tool_use","id":"w","name":"write_file","input":{"path":"never.txt"}}]}),
            json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"w","content":"{\"executed\":false}","is_error":true}]}),
            json!({"role":"assistant","content":[{"type":"tool_use","id":"open","name":"write_file","input":{"path":"also-never.txt"}}]}),
        ]);
        assert!(!paired(&s.messages));
        let p = prepare(&s, "").unwrap().unwrap();
        assert!(p.checkpoint.changed_files.is_empty());
        assert_eq!(p.checkpoint.kept_messages, 0);
        assert!(p.checkpoint.summary.contains("No checks recorded"));
        assert!(paired(&p.messages));
    }
}
