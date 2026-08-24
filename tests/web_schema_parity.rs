//! The web app's option schema, checked against Rust in CI.
//!
//! `web/options.js` already carries `assertSchemaParity` / `assertPresetParity`,
//! which compare its `SCHEMA` and `PRESETS` against the WASM exports at page
//! load. They are the right checks — but they run in a browser and report via
//! `console.error`, and CI executes no JavaScript at all (clippy twice, build,
//! `cargo test`, an advisory offset scan). So the alarm only ever sounded for
//! someone who happened to have devtools open.
//!
//! That is the path by which `boomboom_hits` shipped encoded in the flag key
//! but absent from the web schema (fixed in 8077c5d): the option existed on
//! both sides of the wire, so nothing in Rust noticed, and no one was watching
//! the console. This file mirrors those two checks over the file itself so the
//! same drift fails a build instead.
//!
//! Rust remains the source of truth. This only asserts that the JS agrees with
//! it; when the two disagree, the schema is what changes.
//!
//! **On hand-parsing JavaScript:** the scanners below extract a handful of
//! literal keys from a file with a very regular shape. The real hazard is not a
//! wrong answer but a *vacuous* one — a parser that quietly matches nothing
//! still passes every comparison it is asked to make, which is worse than no
//! check at all because it looks like one. `parser_still_matches_the_file`
//! guards that directly: it asserts every `id: "` in each block is an entry
//! opener, so an entry written any other way fails loudly instead of dropping
//! out of the comparison. Each extractor also asserts it found a plausible
//! number of items before its results are used.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use smb3_rs::{Options, flag_key_fields};

const OPTIONS_JS: &str = "web/options.js";

/// Lower bounds that only a broken parser can trip. They are deliberately far
/// below the real counts — this is a "did the regex stop matching" tripwire,
/// not a second copy of the schema that has to be maintained.
const MIN_SCHEMA_ENTRIES: usize = 40;
const MIN_PRESETS: usize = 3;
const MIN_CONSTANT_FIELDS: usize = 1;

fn options_js() -> String {
    fs::read_to_string(OPTIONS_JS)
        .unwrap_or_else(|e| panic!("{OPTIONS_JS} must be readable from the crate root: {e}"))
}

/// The body of a `export const NAME = [ … ];` array literal.
fn array_block<'a>(src: &'a str, name: &str) -> &'a str {
    let head = format!("export const {name} = [");
    let start = src
        .find(&head)
        .unwrap_or_else(|| panic!("{OPTIONS_JS}: no `{head}` — did the export get renamed?"))
        + head.len();
    let rest = &src[start..];
    let end = rest
        .find("\n];")
        .unwrap_or_else(|| panic!("{OPTIONS_JS}: `{name}` has no closing `\\n];`"));
    &rest[..end]
}

/// Split an array block into one string per `\n\t{ id: ` entry. Every entry in
/// both `SCHEMA` and `PRESETS` is written this way; `parser_still_matches_the
/// _file` fails if that ever stops being true.
fn entries(block: &str) -> Vec<&str> {
    block.split("\n\t{ id: ").skip(1).collect()
}

/// The `"…"` immediately opening an entry.
fn entry_id(entry: &str) -> &str {
    let rest = entry.strip_prefix('"').expect("entry does not open with a quoted id");
    let end = rest.find('"').expect("entry id is not terminated");
    &rest[..end]
}

/// `SCHEMA` as `id -> inFlagKey`. Every entry carries an explicit marking; a
/// missing one is a schema bug, not something to guess a default for.
fn schema() -> BTreeMap<String, bool> {
    let src = options_js();
    let out: BTreeMap<String, bool> = entries(array_block(&src, "SCHEMA"))
        .into_iter()
        .map(|e| {
            let id = entry_id(e);
            let flag = if e.contains("inFlagKey: true") {
                true
            } else if e.contains("inFlagKey: false") {
                false
            } else {
                panic!("{OPTIONS_JS}: SCHEMA entry `{id}` has no `inFlagKey` marking");
            };
            (id.to_string(), flag)
        })
        .collect();
    assert!(
        out.len() >= MIN_SCHEMA_ENTRIES,
        "parsed only {} SCHEMA entries from {OPTIONS_JS} — the parser has almost \
         certainly stopped matching the file, not the schema shrunk",
        out.len(),
    );
    out
}

/// `CONSTANT_FIELDS` — options the app pins rather than exposing a control for.
/// They are real `Options` fields with no schema entry, so the field-set
/// comparison has to know about them.
fn constant_fields() -> BTreeSet<String> {
    let src = options_js();
    let start = src
        .find("const CONSTANT_FIELDS = {")
        .expect("no `const CONSTANT_FIELDS = {` in options.js")
        + "const CONSTANT_FIELDS = {".len();
    let rest = &src[start..];
    let end = rest.find("\n};").expect("CONSTANT_FIELDS has no closing `\\n};`");
    let out: BTreeSet<String> = rest[..end]
        .lines()
        .filter_map(|l| l.trim().split(':').next())
        .map(str::trim)
        .filter(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        .map(str::to_string)
        .collect();
    assert!(
        out.len() >= MIN_CONSTANT_FIELDS,
        "parsed no CONSTANT_FIELDS from {OPTIONS_JS} — parser drift",
    );
    out
}

/// Every `overrides:` key across all presets, tagged with its preset id.
/// Override values are primitives and arrays only — no nested objects and no
/// colons inside strings — which is what makes the flat key scan below safe.
fn preset_overrides() -> Vec<(String, String)> {
    let src = options_js();
    let block = array_block(&src, "PRESETS");
    let presets = entries(block);
    assert!(
        presets.len() >= MIN_PRESETS,
        "parsed only {} presets from {OPTIONS_JS} — parser drift",
        presets.len(),
    );

    let mut out = Vec::new();
    for entry in presets {
        let pid = entry_id(entry);
        let at = entry
            .find("overrides:")
            .unwrap_or_else(|| panic!("preset `{pid}` has no `overrides:`"));
        let open = entry[at..].find('{').expect("overrides has no `{`") + at;

        let mut depth = 0usize;
        let mut close = None;
        for (i, c) in entry[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close.unwrap_or_else(|| panic!("preset `{pid}` overrides never close"));
        // Comments first: `// TODO: revisit` would otherwise read as an override
        // key named `TODO`, failing the build with a message that blames the
        // presets for a parser problem. The JS check ignores comments too.
        let body = strip_comments(&entry[open + 1..close]);
        assert!(
            !body.contains('{'),
            "preset `{pid}` has a nested object in its overrides — the flat key \
             scan in this test no longer describes the file",
        );

        for key in flat_keys(&body) {
            out.push((pid.to_string(), key));
        }
    }
    out
}

/// Drop `//` and `/* … */` comments. String literals are tracked so a `//`
/// inside one (a URL, say) is never mistaken for a comment start.
///
/// Only the result's *content* matters — nothing downstream maps an offset back
/// to the original — so this copies kept spans wholesale rather than trying to
/// preserve length.
fn strip_comments(body: &str) -> String {
    let b = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    let mut kept = 0;
    let mut in_str = false;

    while i < b.len() {
        if in_str {
            match b[i] {
                b'\\' => i += 2,
                b'"' => {
                    in_str = false;
                    i += 1;
                }
                _ => i += 1,
            }
            continue;
        }
        match b[i] {
            b'"' => {
                in_str = true;
                i += 1;
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                out.push_str(&body[kept..i]);
                i += body[i..].find('\n').unwrap_or(body.len() - i);
                kept = i;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                out.push_str(&body[kept..i]);
                i += body[i..].find("*/").map_or(body.len() - i, |e| e + 2);
                kept = i;
            }
            _ => i += 1,
        }
    }
    out.push_str(&body[kept..]);
    out
}

/// Identifiers immediately followed by `:`, skipping anything inside a string
/// literal so a colon in a value can never be read as a key.
fn flat_keys(body: &str) -> Vec<String> {
    let bytes = body.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut in_str = false;
    let mut word_start: Option<usize> = None;

    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' => {
                in_str = true;
                word_start = None;
            }
            // Uppercase counts: a key is only "not an id we know" if we saw it
            // at all, so the scan must accept any identifier shape, not just
            // the lower_snake_case the schema happens to use today.
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'0'..=b'9' => {
                if word_start.is_none() && !c.is_ascii_digit() {
                    word_start = Some(i);
                }
            }
            b':' => {
                if let Some(s) = word_start {
                    out.push(body[s..i].to_string());
                }
                word_start = None;
            }
            _ => word_start = None,
        }
        i += 1;
    }
    out
}

/// Every `Options` field name, as serde spells it on the wire — the same set
/// the WASM `default_options_json()` export ships to the web app.
fn options_fields() -> BTreeSet<String> {
    let value = serde_json::to_value(Options::default()).expect("Options serializes");
    value
        .as_object()
        .expect("Options serializes to an object")
        .keys()
        .cloned()
        .collect()
}

/// The tripwire. Everything else compares sets that this file extracts; if the
/// extraction silently matched nothing, those comparisons would pass while
/// checking nothing at all. Assert the shapes the extractors depend on are
/// still there.
#[test]
fn parser_still_matches_the_file() {
    let src = options_js();
    let schema_entries = entries(array_block(&src, "SCHEMA"));
    let preset_entries = entries(array_block(&src, "PRESETS"));

    assert!(
        schema_entries.len() >= MIN_SCHEMA_ENTRIES,
        "SCHEMA parse found {} entries",
        schema_entries.len(),
    );
    assert!(
        preset_entries.len() >= MIN_PRESETS,
        "PRESETS parse found {} entries",
        preset_entries.len(),
    );

    // Every `id: "…"` in a block must be an entry opener. An entry written any
    // other way is silently skipped by `entries()`, and every check built on it
    // then passes for the wrong reason — a preset reformatted this way takes its
    // whole override list out of the parity check with nothing to show for it.
    for name in ["SCHEMA", "PRESETS"] {
        let block = array_block(&src, name);
        assert_eq!(
            block.matches("id: \"").count(),
            entries(block).len(),
            "{name} has an `id:` that is not an entry opener — `entries()` is \
             under-counting and every check built on it is unreliable",
        );
    }

    assert!(!constant_fields().is_empty(), "CONSTANT_FIELDS parse is empty");
    assert!(!preset_overrides().is_empty(), "preset override parse is empty");
}

/// Mirrors the first half of `assertSchemaParity`: the schema must offer a
/// control for every `Options` field except the ones the app pins itself.
///
/// A field missing here is invisible in the web app — it silently takes its
/// default for every player, which is exactly what happened to `boomboom_hits`.
#[test]
fn schema_covers_every_options_field() {
    let schema: BTreeSet<String> = schema().into_keys().collect();
    let constants = constant_fields();
    let expected: BTreeSet<String> = options_fields().difference(&constants).cloned().collect();

    let missing_in_js: Vec<&String> = expected.difference(&schema).collect();
    let missing_in_rust: Vec<&String> = schema.difference(&expected).collect();

    assert!(
        missing_in_js.is_empty() && missing_in_rust.is_empty(),
        "web/options.js SCHEMA has drifted from Options.\n  \
         in Rust, no web control: {missing_in_js:?}\n  \
         in the schema, not an Options field: {missing_in_rust:?}",
    );
}

/// Mirrors the second half of `assertSchemaParity`: `inFlagKey` must say what
/// Rust actually encodes.
///
/// The marking is not documentation — it drives `applyOptions` and
/// `applyPreset`, so a wrong one means a shared flag key silently fails to
/// apply that option.
#[test]
fn in_flag_key_markings_match_what_rust_encodes() {
    let schema = schema();
    let encoded: BTreeSet<String> = flag_key_fields().into_iter().collect();

    let claimed_but_not_encoded: Vec<&String> = schema
        .iter()
        .filter(|(id, marked)| **marked && !encoded.contains(*id))
        .map(|(id, _)| id)
        .collect();
    let encoded_but_not_claimed: Vec<&String> = schema
        .iter()
        .filter(|(id, marked)| !**marked && encoded.contains(*id))
        .map(|(id, _)| id)
        .collect();

    assert!(
        claimed_but_not_encoded.is_empty() && encoded_but_not_claimed.is_empty(),
        "inFlagKey drift.\n  \
         marked shareable, not in the flag key: {claimed_but_not_encoded:?}\n  \
         in the flag key, not marked: {encoded_but_not_claimed:?}",
    );
}

/// Mirrors `assertPresetParity`: a preset override naming an id that is not a
/// flag-key schema entry is a silent no-op, so the preset quietly stops
/// delivering the setting it advertises.
#[test]
fn preset_overrides_name_live_flag_key_fields() {
    let schema = schema();
    let bad: Vec<String> = preset_overrides()
        .into_iter()
        .filter_map(|(preset, key)| match schema.get(&key) {
            None => Some(format!("{preset}.{key} (not a schema id)")),
            Some(false) => Some(format!("{preset}.{key} (schema id, but inFlagKey: false)")),
            Some(true) => None,
        })
        .collect();

    assert!(bad.is_empty(), "presets reference ids they cannot apply: {bad:?}");
}
