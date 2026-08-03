Generate a ROM for playtesting — specific levels on the map, an overworld seed, or a lock layout.

## Usage
`/test-level <args...>`

The arguments are passed straight through to the `testrom` binary. Common shapes:

- `/test-level --place 6F1` — park 6-F1 on tile 1 of World 1
- `/test-level --place 6F1 5F1 8B` — park three levels on tiles 1, 2, 3
- `/test-level --place 3:8B` — park Bowser's Castle on tile 3 specifically
- `/test-level --place 7A --world 7` — test W7's airship on its home map
- `/test-level --randomize --seed 12345` — playtest a randomized overworld
- `/test-level --randomize --keep-locks --hammer-locks --starting-items hammer,leaf`
  — locks intact, hammer able to break them, walkable map
- `/test-level --randomize --keep-locks` — test lock placement on a real seed
- `/test-level --place-all 7F1` — every numbered level in W1 becomes 7-F1
- `/test-level --list` — show every placeable level name

## Instructions

1. **Build** if needed:
   ```sh
   nix-shell -p gcc --run 'export PATH="$HOME/.cargo/bin:$PATH" && cargo build --bin testrom'
   ```

2. **Run** it with the user's arguments:
   ```sh
   ./target/debug/testrom $ARGUMENTS
   ```

3. **Report** the summary it prints (what was placed where, what was opened up)
   and the output path. Do not re-derive any of it.

That's the whole procedure. All ROM knowledge — level-name resolution, pointer
table surgery, map grid rewrites, the movement patch subset — lives in
`src/testrom.rs` and is covered by unit tests. **Never hand-patch a test ROM
with an inline Python one-liner.** If `testrom` can't express what's needed, add
the flag to the binary rather than working around it.

## Defaults worth knowing

- Base is **vanilla** unless `--randomize` is passed. Use vanilla when testing a
  level's *contents*; use `--randomize` when testing the *overworld* itself.
- The map is **fully open** by default: locks removed, water gaps bridged, and
  open movement patched in so Mario walks over level and fortress tiles without
  entering or clearing them. `--keep-locks` / `--keep-gaps` / `--no-walk` opt out
  individually — pass `--keep-locks` when the locks are the thing under test.
- Level names are case-insensitive with optional dashes: `6F1` == `6-F1`.
  `--beta` adds the 9 unreferenced beta stages to the placeable set.
- `--starting-items` takes up to 3 (the inventory trampoline's slot count);
  `--list-items` shows the valid names. `--hammer-locks` / `--hammer-bridges`
  let the Hammer break lock and water-gap tiles, and work on a vanilla base too.
- Output defaults to `test_level.nes` (`-o` to change).

## When the map layout matters to the test

A random seed may simply not place the feature under test where you need it.
**Use `--require` rather than generating ROMs and checking by hand:**

```sh
./target/debug/testrom --require 'lock@w8:s2' --world 8 --keep-locks --hammer-locks
```

Syntax `<class>@w<N>[:s<M>][>=<K>]` — classes `lock`, `gap`, `fortress`, `level`,
`pipe`, `toadhouse`, `airship`, `bowser`, `tile:0xNN`. It prints the matched
positions and a census of the target screen; **read that output** — a predicate
can pass while the ROM still misses the point (a lock with no fortress beside it
is hammer-breakable, a different code path from a fortress clear).

For W8 darkness specifically the engine gates on `World_Map_XHi == 2`, so only
**screen 2 (columns 32–47)** is dark — hence `lock@w8:s2`.

Run `./target/debug/testrom --help` for the full flag list.
