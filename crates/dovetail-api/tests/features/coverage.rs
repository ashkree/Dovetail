//! Ensures every `Event` variant declared in `src/<domain>/<feature>/event.rs` is exercised by
//! `cases()`. The feature key comes from the enum name (`TaskEvent` is `task`), so a domain
//! folder or a renamed feature folder changes nothing.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use dovetail_api::Event;

fn pascal_to_snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in name.char_indices() {
        if i > 0 && ch.is_ascii_uppercase() {
            out.push('_');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

fn extract_event_variants(path: &Path, source: &str) -> Vec<String> {
    let mut variants = Vec::new();
    let mut feature = String::new();
    let mut in_enum = false;
    let mut depth: usize = 0;

    for line in source.lines() {
        let trimmed = line.trim();
        if !in_enum {
            if trimmed.starts_with("pub enum ")
                && trimmed.contains("Event")
                && trimmed.ends_with('{')
            {
                in_enum = true;
                let name = trimmed
                    .trim_start_matches("pub enum ")
                    .split(' ')
                    .next()
                    .unwrap_or("");
                feature = pascal_to_snake(name.trim_end_matches("Event"));
                depth = 1;
            }
            continue;
        }

        let trimmed_start = line.trim_start();
        let is_comment_or_attr = trimmed_start.starts_with("//") || trimmed_start.starts_with('#');

        if depth == 1
            && !is_comment_or_attr
            && line.starts_with("    ")
            && !line.starts_with("     ")
        {
            let rest = &line[4..];
            if rest.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                let variant: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                assert!(
                    !variant.is_empty(),
                    "failed to parse variant name in {}: {line}",
                    path.display()
                );
                variants.push(format!("{feature}.{}", pascal_to_snake(&variant)));
            }
        }

        if !is_comment_or_attr {
            let code = line.split("//").next().unwrap_or("");
            for ch in code.chars() {
                match ch {
                    '{' => depth += 1,
                    '}' => depth = depth.saturating_sub(1),
                    _ => {}
                }
            }
            if depth == 0 {
                break;
            }
        }
    }

    assert!(
        in_enum,
        "did not find `pub enum <Feature>Event {{` in {}",
        path.display()
    );
    assert!(
        !variants.is_empty(),
        "extracted zero event variants from {}",
        path.display()
    );
    variants
}

fn collect_event_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in
        fs::read_dir(dir).unwrap_or_else(|err| panic!("cannot read {}: {err}", dir.display()))
    {
        let path = entry.expect("readable directory entry").path();
        if path.is_dir() {
            collect_event_files(&path, out);
        } else if path.file_name().is_some_and(|n| n == "event.rs") {
            out.push(path);
        }
    }
}

fn declared_event_keys() -> BTreeSet<String> {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut event_files = Vec::new();
    collect_event_files(&src_dir, &mut event_files);
    event_files.sort();

    let mut keys = BTreeSet::new();
    for path in event_files {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        for key in extract_event_variants(&path, &source) {
            let inserted = keys.insert(key.clone());
            assert!(inserted, "duplicate event key {key}");
        }
    }
    keys
}

fn event_key(event: &Event) -> String {
    // `Event` is a serde-tagged enum with one feature key and one inner variant.
    let value = serde_json::to_value(event).expect("Event always serializes to JSON");
    let obj = value
        .as_object()
        .expect("Event serializes as a JSON object");
    assert_eq!(
        obj.len(),
        1,
        "Event must serialize with a single feature key: {value}"
    );
    let (feature, inner) = obj.iter().next().expect("checked obj.len() == 1");
    let variant = match inner {
        serde_json::Value::String(name) => name.as_str(),
        serde_json::Value::Object(inner_obj) => {
            assert_eq!(
                inner_obj.len(),
                1,
                "nested Event must serialize with a single variant key: {value}"
            );
            inner_obj
                .keys()
                .next()
                .expect("checked inner_obj.len() == 1")
                .as_str()
        }
        other => panic!("unexpected Event JSON payload for {feature}: {other}"),
    };
    format!("{feature}.{variant}")
}

#[test]
fn every_event_variant_has_a_case() {
    let declared = declared_event_keys();
    let tested: BTreeSet<String> = super::all_cases()
        .into_iter()
        .flat_map(|case| case.events)
        .map(|(_, event)| event_key(&event))
        .collect();

    let missing: Vec<&String> = declared.difference(&tested).collect();
    let unexpected: Vec<&String> = tested.difference(&declared).collect();
    assert!(
        missing.is_empty() && unexpected.is_empty(),
        "event coverage mismatch ({} declared, {} tested):\n  missing from cases(): {missing:?}\n  unexpected in cases(): {unexpected:?}",
        declared.len(),
        tested.len(),
    );
}
