//! `flagstats` — turn the flag keys GoatCounter already collected into a
//! per-option census.
//!
//! The web app fires one event per generate at `/generate/<app version>/<flag
//! key>` (`web/app.js`), and a flag key decodes back into the whole `Options`
//! set. So "which flags do players actually use" is a *read* problem rather
//! than a tracking problem — nothing new has to be recorded. This pulls the
//! per-path counts from GoatCounter's API, decodes every key, and tallies each
//! option's values.
//!
//! ```sh
//! export GOATCOUNTER_TOKEN=...                 # [your username] -> API, read-statistics permission
//! cargo run --bin flagstats -- --start 2026-08-06
//! cargo run --bin flagstats -- --start 2026-08-06 --save hits.json
//! cargo run --bin flagstats -- --json hits.json     # re-read, no network
//! ```
//!
//! Three limits are inherent to reading the data this way. All three are
//! reported in the header rather than quietly folded into the percentages:
//!
//! - **Only the current key format decodes.** `Options::from_flag_key` reads
//!   one version; every older key is counted and bucketed by the version it
//!   claims, so partial coverage shows up as a number instead of a bias.
//! - **`NOT_ENCODED` options are invisible.** Palettes, king quotes and the
//!   rest never reach the key — and neither does output format, visual patch,
//!   or which preset was clicked. Those need their own events to be countable.
//! - **GoatCounter counts visitors per path**, not generates: ten seeds rolled
//!   on one ruleset in one session is one hit, and the same ruleset under two
//!   app versions is two paths.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use clap::Parser;
use serde::Deserialize;
use serde_json::Value;

use smb3_rs::{Options, current_flag_key_version, flag_key_fields, flag_key_version_of};

/// Stop growing the `exclude_paths` pagination list past this. Servers reject
/// very long request lines, and the honest failure is "narrow the window",
/// not a 414 from inside a loop.
const MAX_URL_LEN: usize = 6000;

/// Values listed per option before the tail collapses into "+N more". Enough
/// for every enum here; `starting_items` is the one field with a long tail.
const MAX_VALUES_SHOWN: usize = 6;

#[derive(Parser)]
#[command(
    name = "flagstats",
    about = "Census the flag keys GoatCounter recorded, per option",
    long_about = "Decode the flag keys in GoatCounter's /generate/<version>/<key> events into a \
                  per-option census. Needs an API token with 'read statistics' \
                  (GOATCOUNTER_TOKEN, or --token), unless --json reads a saved response."
)]
struct Cli {
    /// GoatCounter site code (<site>.goatcounter.com).
    #[arg(long, default_value = "smb3rs")]
    site: String,

    /// API token with "read statistics". Defaults to $GOATCOUNTER_TOKEN.
    #[arg(long)]
    token: Option<String>,

    /// Start of the window, e.g. 2026-08-06. Keys older than the current
    /// format can't be decoded, so this is how you skip them.
    #[arg(long)]
    start: Option<String>,

    /// End of the window, e.g. 2026-09-01.
    #[arg(long)]
    end: Option<String>,

    /// Read saved API responses instead of fetching. Repeatable.
    #[arg(long, value_name = "FILE")]
    json: Vec<PathBuf>,

    /// Write the fetched responses here, as one merged object --json can read.
    #[arg(long, value_name = "FILE")]
    save: Option<PathBuf>,

    /// Write the census itself as JSON here — the shape the report page
    /// reads. Counts only; no per-visitor anything.
    #[arg(long, value_name = "FILE")]
    census_json: Option<PathBuf>,

    /// Event path prefix the web app uses.
    #[arg(long, default_value = "generate")]
    prefix: String,

    /// Paths per API request.
    #[arg(long, default_value_t = 100)]
    limit: u32,

    /// Exact rulesets to list, most-used first. 0 to skip the section.
    #[arg(long, default_value_t = 10)]
    top: usize,

    /// Print each request URL (never the token) and each page's size.
    #[arg(short, long)]
    verbose: bool,

    /// List options nobody moved off their default too. Off by default: they
    /// are most of the table and all of them say the same thing.
    #[arg(long)]
    all: bool,
}

/// One page of `GET /api/v0/stats/hits`.
#[derive(Deserialize)]
struct HitsResponse {
    #[serde(default)]
    hits: Vec<Hit>,
    #[serde(default)]
    more: bool,
}

/// A path with its count. `count` is visitors for the window, per the API's
/// own definition — see the module header.
#[derive(Deserialize)]
struct Hit {
    path: String,
    #[serde(default)]
    path_id: i64,
    #[serde(default)]
    count: u64,
    #[serde(default)]
    stats: Vec<DayStat>,
}

#[derive(Deserialize)]
struct DayStat {
    day: String,
    #[serde(default)]
    daily: u64,
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(&cli) {
        eprintln!("flagstats: {e}");
        std::process::exit(1);
    }
}

fn run(cli: &Cli) -> Result<(), String> {
    let hits = if cli.json.is_empty() { fetch(cli)? } else { read_saved(&cli.json)? };

    if let Some(path) = &cli.save {
        let merged = serde_json::json!({ "hits": raw_hits(&hits), "more": false });
        std::fs::write(path, serde_json::to_vec_pretty(&merged).map_err(|e| e.to_string())?)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        eprintln!("saved {} paths to {}", hits.len(), path.display());
    }

    let census = census(&hits, &cli.prefix);
    if let Some(path) = &cli.census_json {
        let json = census_json(&census, cli);
        std::fs::write(path, serde_json::to_vec_pretty(&json).map_err(|e| e.to_string())?)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        eprintln!("wrote census JSON to {}", path.display());
    }
    if census.hits == 0 {
        return Err(format!(
            "no '{}/...' event paths in {} paths — wrong --prefix, or an empty window",
            cli.prefix,
            hits.len()
        ));
    }
    report(&census, cli);
    Ok(())
}

/// Re-serialize hits so `--save` output round-trips through `--json`.
fn raw_hits(hits: &[Hit]) -> Vec<Value> {
    hits.iter()
        .map(|h| {
            let days: Vec<Value> = h
                .stats
                .iter()
                .map(|d| serde_json::json!({ "day": d.day, "daily": d.daily }))
                .collect();
            serde_json::json!({
                "path": h.path, "path_id": h.path_id, "count": h.count, "stats": days,
            })
        })
        .collect()
}

// --- Input ------------------------------------------------------------------

fn read_saved(paths: &[PathBuf]) -> Result<Vec<Hit>, String> {
    let mut all = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let resp: HitsResponse =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        all.extend(resp.hits);
    }
    Ok(all)
}

/// Page through `/api/v0/stats/hits` until the API says there is no more.
///
/// Pagination is by exclusion, which is what the API offers: each page's
/// `path_id`s are added to `exclude_paths` so the next page returns what is
/// left. That list is the only thing that grows, hence [`MAX_URL_LEN`].
fn fetch(cli: &Cli) -> Result<Vec<Hit>, String> {
    let token = match cli.token.clone().or_else(|| std::env::var("GOATCOUNTER_TOKEN").ok()) {
        Some(t) if !t.trim().is_empty() => t.trim().to_string(),
        _ => {
            return Err("no API token — set GOATCOUNTER_TOKEN or pass --token (create one \
                        under [your username] -> API with read-statistics permission), or read a \
                        saved response with --json"
                .to_string());
        }
    };
    // The token is handed to curl on stdin, and a curl config file has no
    // escaping to get right — so reject anything that could close the quote
    // instead of trying to quote it.
    if !token.chars().all(|c| c.is_ascii_alphanumeric() || "-_=.~".contains(c)) {
        return Err("API token has characters that are not token characters".to_string());
    }

    let mut hits: Vec<Hit> = Vec::new();
    let mut exclude: Vec<i64> = Vec::new();
    loop {
        let mut url =
            format!("https://{}.goatcounter.com/api/v0/stats/hits?limit={}", cli.site, cli.limit);
        if let Some(start) = &cli.start {
            url.push_str(&format!("&start={start}"));
        }
        if let Some(end) = &cli.end {
            url.push_str(&format!("&end={end}"));
        }
        if !exclude.is_empty() {
            let ids: Vec<String> = exclude.iter().map(i64::to_string).collect();
            url.push_str(&format!("&exclude_paths={}", ids.join(",")));
        }
        if url.len() > MAX_URL_LEN {
            eprintln!(
                "warning: stopped paginating at {} paths (request line got too long) — \
                 narrow the window with --start/--end for the rest",
                hits.len()
            );
            break;
        }

        if cli.verbose {
            eprintln!("GET {url}");
        }
        let body = curl(&url, &token)?;
        let page: HitsResponse = serde_json::from_str(&body).map_err(|e| {
            format!(
                "API response wasn't the expected JSON ({e}): {}",
                body.chars().take(200).collect::<String>()
            )
        })?;
        if page.hits.is_empty() {
            break;
        }
        if cli.verbose {
            eprintln!("  {} paths, more={}", page.hits.len(), page.more);
        }
        exclude.extend(page.hits.iter().map(|h| h.path_id));
        hits.extend(page.hits);
        if !page.more {
            break;
        }
    }
    Ok(hits)
}

/// Run one authenticated GET. Options go in on stdin rather than argv: a bearer
/// token in the process list is readable by every other process on the machine.
fn curl(url: &str, token: &str) -> Result<String, String> {
    let mut child = Command::new("curl")
        .arg("--config")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("running curl: {e}"))?;
    let config = format!(
        "silent\nshow-error\nfail\nurl = \"{url}\"\nheader = \"Authorization: Bearer {token}\"\n"
    );
    child
        .stdin
        .take()
        .ok_or("curl stdin")?
        .write_all(config.as_bytes())
        .map_err(|e| format!("writing curl config: {e}"))?;
    let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("curl failed ({}) — check the token and site code", out.status));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("curl output wasn't UTF-8: {e}"))
}

// --- Census -----------------------------------------------------------------

#[derive(Default)]
struct Census {
    /// Hits on paths that parsed as generate events.
    hits: u64,
    keys: BTreeMap<String, u64>,
    decoded_hits: u64,
    decoded_keys: usize,
    /// Hits whose key this build did not decode, by the version the key
    /// claims. Usually an older format; a *current*-version entry is a key
    /// whose envelope was intact but whose payload was rejected.
    undecoded: BTreeMap<u8, u64>,
    /// Hits whose key didn't decode and didn't name a version either.
    unreadable: u64,
    app_versions: BTreeMap<String, u64>,
    /// field -> value label -> hits.
    fields: BTreeMap<String, BTreeMap<String, u64>>,
    /// Non-default option count per decoded key, for the ruleset list.
    away: BTreeMap<String, usize>,
    /// Generate hits per calendar day, summed across paths.
    daily: BTreeMap<String, u64>,
    /// Decoded hits with the maze on, and how those hits set the two options
    /// that only mean anything in that mode.
    maze_hits: u64,
    maze_fields: BTreeMap<String, BTreeMap<String, u64>>,
    first_day: Option<String>,
    last_day: Option<String>,
}

fn census(hits: &[Hit], prefix: &str) -> Census {
    let mut c = Census::default();
    let defaults = options_map(&Options::default());
    let encoded = flag_key_fields();
    let mut decoded_keys = BTreeMap::new();

    for hit in hits {
        let Some((app_version, key)) = split_event_path(&hit.path, prefix) else { continue };
        c.hits += hit.count;
        *c.keys.entry(key.to_string()).or_default() += hit.count;
        *c.app_versions.entry(app_version.to_string()).or_default() += hit.count;
        for day in &hit.stats {
            if day.daily == 0 {
                continue;
            }
            *c.daily.entry(day.day.clone()).or_default() += day.daily;
            if c.first_day.as_deref().is_none_or(|d| day.day.as_str() < d) {
                c.first_day = Some(day.day.clone());
            }
            if c.last_day.as_deref().is_none_or(|d| day.day.as_str() > d) {
                c.last_day = Some(day.day.clone());
            }
        }

        let decoded = decoded_keys
            .entry(key.to_string())
            .or_insert_with(|| Options::from_flag_key(key).ok().map(|o| options_map(&o)));
        match decoded {
            Some(map) => {
                c.decoded_hits += hit.count;
                let mut away = 0;
                for field in &encoded {
                    let value = map.get(field).unwrap_or(&Value::Null);
                    if defaults.get(field) != Some(value) {
                        away += 1;
                    }
                    let label = value_label(field, value);
                    *c.fields.entry(field.clone()).or_default().entry(label).or_default() +=
                        hit.count;
                }
                c.away.insert(key.to_string(), away);
                // `maze_wands` and `hints` decode to their defaults unless the
                // maze is on, so tallying them over everyone would report
                // "3 wands" for people who never opened the mode. Cohort them.
                if map.get("world_maze") == Some(&Value::Bool(true)) {
                    c.maze_hits += hit.count;
                    for field in ["maze_wands", "hints"] {
                        let value = map.get(field).unwrap_or(&Value::Null);
                        let label = value_label(field, value);
                        *c.maze_fields
                            .entry(field.to_string())
                            .or_default()
                            .entry(label)
                            .or_default() += hit.count;
                    }
                }
            }
            None => match flag_key_version_of(key) {
                Ok(v) => *c.undecoded.entry(v).or_default() += hit.count,
                Err(_) => c.unreadable += hit.count,
            },
        }
    }
    c.decoded_keys = decoded_keys.values().filter(|d| d.is_some()).count();
    c
}

/// `generate/<app version>/<flag key>`, with or without a leading slash —
/// GoatCounter stores event names unslashed, but a saved response from an
/// older export may carry one.
fn split_event_path<'a>(path: &'a str, prefix: &str) -> Option<(&'a str, &'a str)> {
    let rest = path.trim_start_matches('/').strip_prefix(prefix)?.strip_prefix('/')?;
    let (version, key) = rest.split_once('/')?;
    (!version.is_empty() && !key.is_empty()).then_some((version, key))
}

fn options_map(options: &Options) -> BTreeMap<String, Value> {
    let value = serde_json::to_value(options).expect("Options is always serializable");
    match value {
        Value::Object(map) => map.into_iter().collect(),
        _ => BTreeMap::new(),
    }
}

/// A value as one short cell. Item IDs get their names — `starting_items` is
/// the only field whose raw numbers mean nothing on screen.
fn value_label(field: &str, value: &Value) -> String {
    match value {
        Value::Bool(true) => "on".to_string(),
        Value::Bool(false) => "off".to_string(),
        Value::Null => "none".to_string(),
        Value::String(s) => s.clone(),
        Value::Number(n) if field == "starting_items" => item_name(n.as_u64().unwrap_or(0) as u8),
        Value::Number(n) => n.to_string(),
        Value::Array(items) if items.is_empty() => "none".to_string(),
        Value::Array(items) => {
            items.iter().map(|v| value_label(field, v)).collect::<Vec<_>>().join("+")
        }
        other => other.to_string(),
    }
}

/// Item ID to its CLI name, via the same table the parsers use.
fn item_name(id: u8) -> String {
    smb3_rs::ITEMS
        .iter()
        .find(|&&(_, item, _)| item == id)
        .map_or_else(|| format!("item{id}"), |&(name, _, _)| name.to_string())
}

/// The census as one JSON object — the document the report page reads.
///
/// Counts only: this is aggregate-of-aggregates (GoatCounter hands out visitors
/// per path, never a visitor), so there is nothing per-person in here to leak.
/// The timestamp is epoch seconds rather than a formatted date, which keeps date
/// formatting on the side that has a locale.
fn census_json(c: &Census, cli: &Cli) -> Value {
    let defaults = options_map(&Options::default());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    let value_rows = |field: &str, values: &BTreeMap<String, u64>| -> Vec<Value> {
        let mut cells: Vec<(&String, u64)> = values.iter().map(|(l, n)| (l, *n)).collect();
        cells.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let default = defaults.get(field).map(|v| value_label(field, v));
        cells
            .into_iter()
            .map(|(label, hits)| {
                serde_json::json!({
                    "label": label, "hits": hits,
                    "is_default": Some(label) == default.as_ref(),
                })
            })
            .collect()
    };

    let mut options: Vec<Value> = c
        .fields
        .iter()
        .map(|(field, values)| {
            let default = defaults.get(field).map(|v| value_label(field, v));
            let away: u64 = values
                .iter()
                .filter(|(label, _)| Some(*label) != default.as_ref())
                .map(|(_, n)| *n)
                .sum();
            serde_json::json!({
                "field": field,
                "default": default,
                "away": away,
                "values": value_rows(field, values),
            })
        })
        .collect();
    options.sort_by(|a, b| {
        let away = |v: &Value| v["away"].as_u64().unwrap_or(0);
        away(b).cmp(&away(a)).then_with(|| a["field"].as_str().cmp(&b["field"].as_str()))
    });

    let maze_options: Vec<Value> = c
        .maze_fields
        .iter()
        .map(|(field, values)| {
            serde_json::json!({ "field": field, "values": value_rows(field, values) })
        })
        .collect();

    let mut rulesets: Vec<(&String, u64)> = c.keys.iter().map(|(k, n)| (k, *n)).collect();
    rulesets.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let rulesets: Vec<Value> = rulesets
        .into_iter()
        .take(25)
        .map(|(key, hits)| serde_json::json!({ "key": key, "hits": hits, "away": c.away.get(key) }))
        .collect();

    serde_json::json!({
        "site": cli.site,
        "generated_at_unix": now,
        "key_format": current_flag_key_version(),
        "window": {
            "requested_start": cli.start,
            "requested_end": cli.end,
            "first_day": c.first_day,
            "last_day": c.last_day,
        },
        "totals": {
            "hits": c.hits,
            "keys": c.keys.len(),
            "decoded_hits": c.decoded_hits,
            "decoded_keys": c.decoded_keys,
            "undecoded_hits": c.undecoded.values().sum::<u64>(),
            "unreadable_hits": c.unreadable,
        },
        "undecoded_by_version": c.undecoded.iter().rev()
            .map(|(v, n)| serde_json::json!({ "version": v, "hits": n }))
            .collect::<Vec<_>>(),
        "app_versions": sorted_counts(&c.app_versions),
        "daily": c.daily.iter()
            .map(|(day, hits)| serde_json::json!({ "day": day, "hits": hits }))
            .collect::<Vec<_>>(),
        "options": options,
        "maze_cohort": { "hits": c.maze_hits, "options": maze_options },
        "rulesets": rulesets,
    })
}

/// `{name, hits}` rows, busiest first.
fn sorted_counts(counts: &BTreeMap<String, u64>) -> Vec<Value> {
    let mut entries: Vec<(&String, u64)> = counts.iter().map(|(k, n)| (k, *n)).collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    entries.iter().map(|(name, n)| serde_json::json!({ "name": name, "hits": n })).collect()
}

// --- Report -----------------------------------------------------------------

fn report(c: &Census, cli: &Cli) {
    let window = match (&c.first_day, &c.last_day) {
        (Some(a), Some(b)) => format!("{a} .. {b}"),
        _ => "unknown window".to_string(),
    };
    println!("GoatCounter flag census — {} ({window})", cli.site);
    println!(
        "  {} hits on {} distinct flag keys — GoatCounter counts visitors per path, not generates",
        c.hits,
        c.keys.len()
    );
    println!(
        "  decoded {} hits / {} keys as format v{}",
        c.decoded_hits,
        c.decoded_keys,
        current_flag_key_version()
    );
    if !c.undecoded.is_empty() {
        let list: Vec<String> =
            c.undecoded.iter().rev().map(|(v, n)| format!("v{v} x{n}")).collect();
        println!("  not decoded, by the version the key claims: {}", list.join(", "));
        if c.undecoded.contains_key(&current_flag_key_version()) {
            println!(
                "    (a v{} entry is this build's own format with a payload it rejected, \
                 not an old key)",
                current_flag_key_version()
            );
        }
    }
    if c.unreadable > 0 {
        println!("  unreadable keys: {}", c.unreadable);
    }
    println!("  app versions: {}", top_list(&c.app_versions, 8));

    if c.decoded_hits == 0 {
        println!(
            "\nNothing decoded — pass --start on or after the day the current key \
                  format shipped."
        );
        return;
    }

    println!(
        "\nPer-option split of {} decoded hits, most-changed first (* = default)",
        c.decoded_hits
    );
    let defaults = options_map(&Options::default());
    let width = c.fields.keys().map(String::len).max().unwrap_or(0);
    let mut rows: Vec<(&String, &BTreeMap<String, u64>, u64)> = c
        .fields
        .iter()
        .map(|(field, values)| {
            let default = defaults.get(field).map(|v| value_label(field, v));
            let away: u64 = values
                .iter()
                .filter(|(label, _)| Some(*label) != default.as_ref())
                .map(|(_, n)| *n)
                .sum();
            (field, values, away)
        })
        .collect();
    rows.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(b.0)));
    let untouched = rows.iter().filter(|(_, _, away)| *away == 0).count();
    if !cli.all {
        rows.retain(|(_, _, away)| *away > 0);
    }

    for (field, values, _) in rows {
        let default = defaults.get(field).map(|v| value_label(field, v));
        let mut cells: Vec<(&String, u64)> = values.iter().map(|(l, n)| (l, *n)).collect();
        cells.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let shown: Vec<String> = cells
            .iter()
            .take(MAX_VALUES_SHOWN)
            .map(|(label, n)| {
                let star = if Some(*label) == default.as_ref() { "*" } else { "" };
                format!("{label}{star} {n} {}%", pct(*n, c.decoded_hits))
            })
            .collect();
        let tail = cells.len().saturating_sub(MAX_VALUES_SHOWN);
        let more = if tail > 0 { format!(" · +{tail} more") } else { String::new() };
        println!("  {field:<width$}  {}{more}", shown.join(" · "));
    }

    if untouched > 0 && !cli.all {
        println!(
            "  ({untouched} more options sat at their default in every hit — --all lists them)"
        );
    }

    if cli.top > 0 {
        println!("\nTop {} exact rulesets", cli.top);
        let mut keys: Vec<(&String, u64)> = c.keys.iter().map(|(k, n)| (k, *n)).collect();
        keys.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        for (key, n) in keys.into_iter().take(cli.top) {
            let shape = match c.away.get(key) {
                Some(0) => " (all defaults)".to_string(),
                Some(away) => format!(" ({away} options off default)"),
                None => " (older key format)".to_string(),
            };
            println!("  {n:>6}  {key}{shape}");
        }
    }
}

fn pct(n: u64, total: u64) -> u64 {
    if total == 0 { 0 } else { (n * 200 + total) / (total * 2) }
}

/// `name xN` for the busiest entries, comma-separated.
fn top_list(counts: &BTreeMap<String, u64>, limit: usize) -> String {
    let mut entries: Vec<(&String, u64)> = counts.iter().map(|(k, n)| (k, *n)).collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let shown: Vec<String> =
        entries.iter().take(limit).map(|(name, n)| format!("{name} x{n}")).collect();
    let tail = entries.len().saturating_sub(limit);
    if tail > 0 { format!("{}, +{tail} more", shown.join(", ")) } else { shown.join(", ") }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the response shape the API returns, so the test exercises the same
    /// parse the real fetch does.
    fn hit(path: &str, count: u64, id: i64) -> Hit {
        Hit {
            path: path.to_string(),
            path_id: id,
            count,
            stats: vec![DayStat { day: "2026-09-01".to_string(), daily: count }],
        }
    }

    #[test]
    fn event_paths_split_with_or_without_a_leading_slash() {
        assert_eq!(
            split_event_path("generate/2.0.1/SMB3R-ABC", "generate"),
            Some(("2.0.1", "SMB3R-ABC"))
        );
        assert_eq!(
            split_event_path("/generate/2.0.1/SMB3R-ABC", "generate"),
            Some(("2.0.1", "SMB3R-ABC"))
        );
        // A plain pageview, a truncated event, and another site's event all miss.
        assert_eq!(split_event_path("/index.html", "generate"), None);
        assert_eq!(split_event_path("generate/2.0.1", "generate"), None);
        assert_eq!(split_event_path("generated/2.0.1/x", "generate"), None);
    }

    #[test]
    fn a_real_key_tallies_onto_its_own_options() {
        let options = Options { world_maze: true, starting_lives: 99, ..Default::default() };
        let key = options.to_flag_key();
        let c = census(&[hit(&format!("generate/2.0.1/{key}"), 10, 1)], "generate");

        assert_eq!(c.hits, 10);
        assert_eq!(c.decoded_hits, 10);
        assert_eq!(c.decoded_keys, 1);
        assert_eq!(c.fields["world_maze"].get("on"), Some(&10));
        assert_eq!(c.fields["starting_lives"].get("99"), Some(&10));
        // Untouched options are still counted, at their default value.
        assert_eq!(c.fields["powerups"].get("on"), Some(&10));
        // Two of them moved, and the ruleset list reports that count.
        assert!(c.away[&key] >= 2);
        assert_eq!(c.app_versions["2.0.1"], 10);
        assert_eq!(c.first_day.as_deref(), Some("2026-09-01"));
    }

    #[test]
    fn the_same_key_under_two_app_versions_is_one_ruleset() {
        let key = Options::default().to_flag_key();
        let c = census(
            &[
                hit(&format!("generate/2.0.0/{key}"), 3, 1),
                hit(&format!("generate/2.0.1/{key}"), 4, 2),
            ],
            "generate",
        );
        assert_eq!(c.keys[&key], 7);
        assert_eq!(c.decoded_keys, 1);
        assert_eq!(c.away[&key], 0);
    }

    /// An older-format key must land in the version bucket rather than being
    /// dropped — partial coverage has to be visible in the header, since the
    /// percentages below it are computed over the decoded hits alone.
    #[test]
    fn an_older_key_format_is_bucketed_by_version_not_dropped() {
        // Pre-v29 keys were always 13 bytes with the version in byte 0 and no
        // checksum, which is exactly what `flag_key_version_of` still reads.
        let legacy = [28u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let encoded = base32::encode(base32::Alphabet::Crockford, &legacy);
        let key = format!("SMB3R-{encoded}");
        assert_eq!(flag_key_version_of(&key), Ok(28), "crafted key must read as v28");

        let c = census(
            &[hit(&format!("generate/2.0.1/{key}"), 5, 1), hit("generate/2.0.1/SMB3R-NOPE", 2, 2)],
            "generate",
        );
        assert_eq!(c.hits, 7);
        assert_eq!(c.decoded_hits, 0);
        assert_eq!(c.undecoded.get(&28), Some(&5));
        assert_eq!(c.unreadable, 2);
        assert!(c.fields.is_empty(), "nothing decoded, so no option got a tally");
    }

    #[test]
    fn item_ids_are_labelled_by_name() {
        let items = Value::Array(vec![Value::from(0x0Bu8), Value::from(0x03u8)]);
        assert_eq!(value_label("starting_items", &items), "hammer+leaf");
        assert_eq!(value_label("starting_items", &Value::Array(vec![])), "none");
        // Every other numeric field keeps its number.
        assert_eq!(value_label("starting_lives", &Value::from(99u8)), "99");
    }
}
