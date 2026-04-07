Review a new or modified 6502 ROM patch for correctness and safety.

## Usage
`/review-patch [description of the patch or file being modified]`

## Instructions

Perform the following checks and report findings:

### 1. Free Space Audit
- Read `src/randomize/rom_data.rs` and find the `FREE_SPACE_ALLOCATIONS` registry (near top of file)
- List all current allocations grouped by PRG bank
- For the patch under review, verify:
  - The file offset is registered in `FREE_SPACE_ALLOCATIONS` with correct size
  - There is a corresponding `pub(super) const FS_*` constant
  - The module uses the `FS_*` constant (not a local hardcoded offset)
  - The size in the registry matches or exceeds the actual bytes written

### 2. Overlap Check
- Confirm `cargo test free_space` passes (this runs the overlap and constant-match tests)
- If tests aren't passing, identify which allocations conflict

### 3. Patch Site Review
- For hook sites (where existing code is overwritten with JMP/JSR):
  - Verify the original instruction size matches the replacement (e.g., 3-byte JSR replaced with 3-byte JMP)
  - Check that the hook doesn't clobber adjacent instructions
  - Verify the bank is correct (CPU address → file offset mapping)
- For subroutines in free space:
  - Verify the CPU address calculation: `cpu_base + (file_offset - bank_file_base)`
  - Bank file bases: PRG000=0x00010, PRG001=0x02010, PRG010=0x14010, PRG011=0x16010, PRG012=0x18010, PRG024=0x30010, PRG026=0x34010, PRG027=0x36010, PRG030=0x3C010, PRG031=0x3E010
  - CPU bases: $8000 for even banks, $A000 for odd banks, $C000 for PRG030, $E000 for PRG031

### 4. Known Danger Zones
Flag if the patch writes to any of these ranges:
- **0x19100–0x19DCF** (PRG012): active tile lookup + map screen code. Writing here crashes on level entry.
- **Bank 24 (PRG024)**: avoid JMP-to-free-space — shares code paths with 2P mode, causes switching bugs. Prefer inline patches.

### 5. Ordering Concerns
- If the patch touches pointer tables or airship entries, check ordering relative to autoscroll and overworld builder in `randomizer.rs`
- Autoscroll MUST run before overworld builder (writes to hardcoded vanilla offsets that get displaced by resort_pointer_table)

### 6. Summary
Report: pass/fail for each check, any warnings, and suggested fixes.
