//! Reading what a native `claude` run added to a session's transcript (take-over, M14).
//!
//! Claude Code files every session as JSONL (`<home>/projects/<cwd>/<id>.jsonl`), one line per
//! message or content block. After a take-over the core reads the lines the native run
//! appended and shows them in the conversation. Only what is cheap and certain is taken: the
//! user's typed messages, assistant text, and tool names. Meta lines (caveats, slash-command
//! echoes), subagent (sidechain) lines, tool results and anything unknown are skipped, never an
//! error (stream rule 5 holds for the transcript too).

use std::io::{self, BufRead, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

/// One thing the native run added to the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeEntry {
    /// A message the user typed.
    User(String),
    /// Assistant text.
    Assistant(String),
    /// A tool the assistant called, by name.
    ToolUse(String),
}

/// The transcript's length now: where a later [`read_since`] starts. A file not there yet is
/// empty.
pub fn len(path: &Path) -> io::Result<u64> {
    match std::fs::metadata(path) {
        Ok(meta) => Ok(meta.len()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e),
    }
}

/// The entries in the lines written at or after byte `offset`, in order. A missing file has
/// none. A partial last line (still being written) is left out.
pub fn read_since(path: &Path, offset: u64) -> io::Result<Vec<NativeEntry>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    // A transcript shorter than the offset was rewritten: read it all.
    let start = if file.metadata()?.len() < offset {
        0
    } else {
        offset
    };
    file.seek(SeekFrom::Start(start))?;
    let mut out = Vec::new();
    let mut reader = io::BufReader::new(file);
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        if !line.ends_with('\n') {
            break;
        }
        out.extend(parse_line(&line));
    }
    Ok(out)
}

/// The entries in one transcript line.
pub fn parse_line(line: &str) -> Vec<NativeEntry> {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    let flag = |key: &str| v.get(key).and_then(Value::as_bool).unwrap_or(false);
    if flag("isMeta") || flag("isSidechain") {
        return Vec::new();
    }
    let content = v.get("message").and_then(|m| m.get("content"));
    match v.get("type").and_then(Value::as_str) {
        Some("user") => user_text(content)
            .map(NativeEntry::User)
            .into_iter()
            .collect(),
        Some("assistant") => content
            .and_then(Value::as_array)
            .map(|blocks| blocks.iter().filter_map(assistant_block).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// A user line's typed text: a plain string, or its text blocks. Tool results and the echoes
/// of slash commands (`<command-name>`, `<local-command-stdout>`) are not typed text.
fn user_text(content: Option<&Value>) -> Option<String> {
    let text = match content? {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => {
            if blocks
                .iter()
                .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            {
                return None;
            }
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n\n")
        }
        _ => return None,
    };
    let trimmed = text.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("<command-")
        || trimmed.starts_with("<local-command")
    {
        return None;
    }
    Some(text)
}

fn assistant_block(block: &Value) -> Option<NativeEntry> {
    match block.get("type").and_then(Value::as_str)? {
        "text" => block
            .get("text")
            .and_then(Value::as_str)
            .filter(|t| !t.trim().is_empty())
            .map(|t| NativeEntry::Assistant(t.to_owned())),
        "tool_use" => block
            .get("name")
            .and_then(Value::as_str)
            .map(|n| NativeEntry::ToolUse(n.to_owned())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINES: &str = concat!(
        r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"Caveat: local commands"}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"<command-name>/model</command-name>"}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"fix the test"}}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"m1","role":"assistant","content":[{"type":"thinking","thinking":"hm"}]}}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"m1","role":"assistant","content":[{"type":"text","text":"Looking."},{"type":"tool_use","id":"t1","name":"Bash","input":{}}]}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#,
        "\n",
        r#"{"type":"assistant","isSidechain":true,"message":{"role":"assistant","content":[{"type":"text","text":"subagent text"}]}}"#,
        "\n",
        r#"{"type":"summary","summary":"Fixing tests"}"#,
        "\n",
        "not json\n",
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Fixed."}]}}"#,
        "\n",
    );

    #[test]
    fn typed_text_assistant_text_and_tool_names_are_kept_and_the_rest_skipped() {
        let entries: Vec<NativeEntry> = LINES.lines().flat_map(parse_line).collect();
        assert_eq!(
            entries,
            vec![
                NativeEntry::User("fix the test".into()),
                NativeEntry::Assistant("Looking.".into()),
                NativeEntry::ToolUse("Bash".into()),
                NativeEntry::Assistant("Fixed.".into()),
            ]
        );
    }

    #[test]
    fn only_lines_after_the_offset_are_read_and_a_partial_line_waits() {
        let dir = std::env::temp_dir().join(format!("dovetail-transcript-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("s.jsonl");
        let before = "{\"type\":\"user\",\"message\":{\"content\":\"headless turn\"}}\n";
        std::fs::write(&path, before).expect("write");
        let offset = len(&path).expect("len");
        let after = format!(
            "{before}{}\n{{\"type\":\"user\",\"message\":{{\"content\":\"half",
            r#"{"type":"user","message":{"content":"native turn"}}"#
        );
        std::fs::write(&path, after).expect("write");
        assert_eq!(
            read_since(&path, offset).expect("read"),
            vec![NativeEntry::User("native turn".into())]
        );
        assert_eq!(
            read_since(&dir.join("missing.jsonl"), 0).expect("missing"),
            vec![]
        );
        assert_eq!(len(&dir.join("missing.jsonl")).expect("len"), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
