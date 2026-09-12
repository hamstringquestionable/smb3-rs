//! Turn preset flag keys into the sparse JS override maps `web/options.js`
//! wants.
//!
//! Revising a preset means deciding the options in the app, copying the flag
//! key, and then writing the same settings a second time as a `{field: value}`
//! map — by hand, against `Options::default()`, for 60-odd fields. That second
//! step is what this does:
//!
//! ```sh
//! cargo run --example preset_decode -- SMB3R-3QMZZZVZ3SGT8AK9WBXQQKR ...
//! ```
//!
//! For each key it prints the fields that differ from `Options::default()`,
//! formatted as a JS object body ready to paste into an `overrides:` block, and
//! **re-encodes the decoded options to check they produce the key it was
//! given** — so a key that was mistyped or came from an older flag-key version
//! is reported rather than silently yielding a plausible-looking map.
//!
//! Why the map and not the key is what `options.js` stores: a preset keyed by
//! stable field ids survives a change to the bit layout, where a stored key
//! would decode to something else entirely. See the `PRESETS` comment there.

use smb3_rs::Options;

fn main() {
    let keys: Vec<String> = std::env::args().skip(1).collect();
    if keys.is_empty() {
        eprintln!(
            "usage: cargo run --example preset_decode -- <flag key> [<flag key> ...]\n\n\
             Prints each key's options as a JS `overrides:` body, and checks the key\n\
             round-trips. Paste the output into `PRESETS` in web/options.js."
        );
        std::process::exit(2);
    }

    let default = serde_json::to_value(Options::default()).expect("serialize default options");
    let default = default.as_object().expect("options serialize to an object");
    let mut failed = false;

    for key in keys {
        let opts = match Options::from_flag_key(&key) {
            Ok(o) => o,
            Err(e) => {
                println!("// {key}\n// DECODE FAILED: {e}\n");
                failed = true;
                continue;
            }
        };

        // The one check worth making automatically: the key we were handed has
        // to be the key these options encode to. A key from an older flag-key
        // version can still decode into *something*, and that something would
        // become a preset nobody asked for.
        let round = opts.to_flag_key();
        if round != key {
            println!("// {key}\n// RE-ENCODES TO A DIFFERENT KEY: {round}");
            println!("// Stale flag-key version, or a typo — do not paste this map.\n");
            failed = true;
            continue;
        }

        let json = serde_json::to_value(&opts).expect("serialize options");
        let mut diff: Vec<(&String, &serde_json::Value)> = json
            .as_object()
            .expect("options serialize to an object")
            .iter()
            .filter(|(k, v)| default.get(*k) != Some(*v))
            .collect();
        diff.sort_by_key(|(k, _)| (*k).clone());

        println!("\t\t// decoded from {key}");
        println!("\t\toverrides: {{");
        for (k, v) in diff {
            println!("\t\t\t{k}: {},", serde_json::to_string(v).expect("serialize value"));
        }
        println!("\t\t}} }},\n");
    }

    if failed {
        std::process::exit(1);
    }
}
