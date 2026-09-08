# `docs/` — index

Every design doc here, and **what it is worth trusting for**. The docs in this
repo are a mix of live design authority, historical RFCs, and one-session
findings, and telling them apart used to require reading each one to the end.
This index says which is which; each historical doc also carries a banner at
its own top.

Status column:

- **Live** — describes what the code does today. If it disagrees with the code,
  that is a bug in the doc and worth fixing.
- **Reference** — ROM facts, not design. Vanilla geometry does not rot.
- **Historical** — a plan, an RFC, or a session's findings, kept for the record.
  **Do not read as a description of current behavior.** Each has a banner.

| Doc | Status | What it is |
|---|---|---|
| [vision.md](vision.md) | **Live** | Why the project exists, who it serves, what it refuses to be. The north star a feature argument is settled against. |
| [smb3_rom_reference.md](smb3_rom_reference.md) | **Reference** | The ROM-hacking reference: offsets, data structures, RAM map, bank layout. **New ROM findings get recorded here** (CLAUDE.md's rule). Sections describing subsystems the randomizer has since replaced carry vanilla-only banners. |
| [application_flow.md](application_flow.md) | **Live** | Every randomization step in the order `randomize_inner` applies them, with its gate and write-log tag. Regenerate from the function when it drifts. |
| [choice_first_charter.md](choice_first_charter.md) | **Live** | The overworld builder's design authority — the point, the values, the pipeline, the measured baselines. |
| [world_maze_design.md](world_maze_design.md) | **Live** | The world maze: eight worlds linked by telepads. Shipped to `beta/next` 2026-09-07; **never playtested end to end.** Several mid-document sections are marked superseded by the fortress-FX rework — read the banners. |
| [fx_table_redesign.md](fx_table_redesign.md) | **Live** | Why vanilla's 17 fortress-FX slots were retired for one position-keyed table, and how. The mechanism is `lock_keys.rs`. |
| [big_q_rooms_design.md](big_q_rooms_design.md) | **Live** | The Big [?] bonus-room shuffle, opt-in since PR #197. Well maintained; the pool is 19 rooms. |
| [wild_injection_rework.md](wild_injection_rework.md) | **Live** | How wild enemy injection works after the 0.12.2 rework. Verified accurate. |
| [write_log_design.md](write_log_design.md) | **Live** | The ROM write log and the free-space auditor. Enhancements 1 and 2 are built; 3 is still design. |
| [overworld_baseline_log.md](overworld_baseline_log.md) | **Live** | The overworld baseline recapture log. Restarted 2026-09-08, when the baseline began hashing the overworld rather than the whole ROM. |
| [start_airship_swap_findings.md](start_airship_swap_findings.md) | **Reference** | Engine internals behind the start ↔ airship swap. Verified against the disassembly. |
| [seed_report_design.md](seed_report_design.md) | **Design note** | A spoiler log. **Nothing is implemented** — the doc says so itself. |
| [overworld_shuffle_logic.md](overworld_shuffle_logic.md) | **Historical** | The pre-rebuild overworld design (2026-02). §3 and §10 are still-good ROM geometry; §§5-6, 9 describe an architecture that was never built. |
| [palette_randomizer_design.md](palette_randomizer_design.md) | **Historical** | An RFC frozen at the moment "Option B" was picked. What shipped diverged in coverage, technique and options — `palettes.rs`'s doc comments are the live account. |
| [pipe_swap_poc_findings.md](pipe_swap_poc_findings.md) | **Historical** | A February POC whose module and CLI flag were deleted. Superseded by the pipe-shuffle section of the ROM reference. |

## Keeping this honest

A doc goes stale in one of two ways, and they need different fixes:

- **A number drifted** (free space, a call-site count, a census percentage).
  Do not hand-copy those figures in the first place — cite the command or the
  registry row that produces them (`smb3-rs <rom> --free-space`,
  `FREE_SPACE_ALLOCATIONS`).
- **A mechanism was replaced.** A banner at the top of the *section that is now
  wrong* beats a note at the bottom of the document, because the reader who
  needs it arrives in the middle. When a doc reverses an earlier decision, go
  back and mark the paragraph that said the opposite — most of the
  contradictions found in the 2026-09-08 sweep were exactly one day of drift
  between a design note and the commit that obsoleted it.
