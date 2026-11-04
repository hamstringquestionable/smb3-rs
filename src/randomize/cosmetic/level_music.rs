//! Random level music: each level gets its own song remap.
//!
//! Header byte 8 bits 0-3 index `GamePlay_BGM` (PRG030 `$97AC`), which
//! `LevelLoad` queues on entry. A junction runs `LevelLoad` again on the new
//! area's header (`Level_JctInit` jumps back to `PRG030_897B`), so every area
//! of a level carries its own song.
//!
//! Each level is dealt a remap of [`POOL`] rather than a song per header:
//! every distinct pool song in the level gets a distinct replacement, so a
//! level stays consistent with itself and an area that sounded different
//! from the rest (1-1's bonus room) still does. Levels that share a header
//! (Hammer Bro arenas, a level on two tiles) are dealt as one. Songs outside
//! [`POOL`] keep their place: those songs tell the player what the room is.
//!
//! Levels are found through [`level_index`], never by scanning, and a header
//! whose bytes overlap another area (vanilla's 3HB18) is never written.
//!
//! Draws from its own substream ([`MUSIC_SALT`]), so the option moves no other
//! part of the seed, and off writes nothing.

use std::collections::{BTreeMap, BTreeSet};

use rand::SeedableRng;
use rand::seq::SliceRandom;
use rand_chacha::ChaCha8Rng;

use crate::randomize::rom_data::level_index;
use crate::rom::Rom;

/// Salt for the music substream.
const MUSIC_SALT: u64 = 0x4D55_5349_435F_5631; // "MUSIC_V1"

/// `GamePlay_BGM` indexes that are rerolled, and what they are rerolled to:
/// overworld, underground, water, fortress, airship, athletic, sky. Left out
/// on purpose: boss (4), Hammer Bro battle (6), Toad House (7), throne room
/// (9).
const POOL: [u8; 7] = [0, 1, 2, 3, 5, 8, 10];

/// Deal every level a remap of [`POOL`] and write it to its headers.
pub(crate) fn randomize(rom: &mut Rom, seed: u64) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ MUSIC_SALT);
    for headers in level_groups(rom) {
        let mut deal = POOL;
        deal.shuffle(&mut rng);
        let mut remap = BTreeMap::new();
        for &h in &headers {
            let b8 = rom.read_byte(h + 8);
            let song = b8 & 0x0F;
            if POOL.contains(&song) {
                let next = remap.len();
                let new = *remap.entry(song).or_insert(deal[next]);
                rom.write_byte(h + 8, (b8 & 0xF0) | new);
            }
        }
    }
}

/// The header offsets of each level, levels that share a header merged into
/// one group. Each group lists its headers in the order the engine reaches
/// them (main area first); the groups are in order of their first header.
/// Headers that overlap another area's bytes are left out.
fn level_groups(rom: &Rom) -> Vec<Vec<usize>> {
    let levels = level_index::every_level(rom);

    // Sweep the areas in file order: one that starts inside an earlier real
    // area's bytes (header through terminator) is that area's data, not a
    // header, and its own span does not count against the areas after it.
    let spans: BTreeSet<(usize, usize)> =
        levels.iter().flatten().map(|a| (a.header.offset, a.terminator)).collect();
    let mut writable = BTreeSet::new();
    let mut end_of_real = None;
    for &(start, end) in &spans {
        if end_of_real.is_none_or(|e| start > e) {
            writable.insert(start);
            end_of_real = Some(end);
        }
    }
    let writable = |h: usize| writable.contains(&h);

    // Merge levels that share a header: `owner` maps each header to the
    // first level that reached it, and `group` points each level at the
    // level it was merged into.
    let mut group: Vec<usize> = (0..levels.len()).collect();
    let find = |group: &[usize], mut i: usize| {
        while group[i] != i {
            i = group[i];
        }
        i
    };
    let mut owner: BTreeMap<usize, usize> = BTreeMap::new();
    for (i, areas) in levels.iter().enumerate() {
        for a in areas {
            let first = *owner.entry(a.header.offset).or_insert(i);
            let (root, other) = (find(&group, i), find(&group, first));
            group[root] = other;
        }
    }

    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, areas) in levels.iter().enumerate() {
        let headers = groups.entry(find(&group, i)).or_default();
        for a in areas {
            let h = a.header.offset;
            if writable(h) && !headers.contains(&h) {
                headers.push(h);
            }
        }
    }
    let mut groups: Vec<Vec<usize>> = groups.into_values().filter(|g| !g.is_empty()).collect();
    groups.sort_by_key(|g| *g.iter().min().unwrap());
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_rom() -> Option<Rom> {
        let data = std::fs::read("roms/Super Mario Bros. 3 (USA) (Rev 1).nes").ok()?;
        Rom::from_bytes(&data).ok()
    }

    /// Only the song nibble of a writable header changes, and only from one
    /// pool song to another; timers and the kept songs stay.
    #[test]
    fn only_pool_song_nibbles_change() {
        let Some(vanilla) = load_rom() else { return };
        let mut rom = vanilla.clone();
        randomize(&mut rom, 1);
        let song_bytes: BTreeSet<usize> =
            level_groups(&vanilla).into_iter().flatten().map(|h| h + 8).collect();
        let mut changed = 0;
        for i in 0..vanilla.data.len() {
            let (a, b) = (vanilla.data[i], rom.data[i]);
            if a == b {
                continue;
            }
            changed += 1;
            assert!(song_bytes.contains(&i), "0x{i:05X} is not a header's byte 8");
            assert_eq!(a & 0xF0, b & 0xF0, "0x{i:05X}: timer bits moved");
            assert!(POOL.contains(&(a & 0x0F)) && POOL.contains(&(b & 0x0F)));
        }
        assert!(changed > 0, "nothing was rerolled");
    }

    /// Within a level, two areas share a song after the deal exactly when
    /// they shared one before it.
    #[test]
    fn each_level_keeps_its_song_pattern() {
        let Some(vanilla) = load_rom() else { return };
        for seed in 0..20 {
            let mut rom = vanilla.clone();
            randomize(&mut rom, seed);
            for headers in level_groups(&vanilla) {
                let song = |r: &Rom, h: usize| r.read_byte(h + 8) & 0x0F;
                for &a in &headers {
                    for &b in &headers {
                        assert_eq!(
                            song(&vanilla, a) == song(&vanilla, b),
                            song(&rom, a) == song(&rom, b),
                            "seed {seed}: headers 0x{a:05X} and 0x{b:05X}"
                        );
                    }
                }
            }
        }
    }

    /// 3HB18's header sits on top of the next area's, so it is never dealt.
    #[test]
    fn the_overlapping_3hb18_header_is_never_written() {
        let Some(rom) = load_rom() else { return };
        let headers: BTreeSet<usize> = level_groups(&rom).into_iter().flatten().collect();
        assert!(!headers.contains(&0x213F7));
        assert!(headers.contains(&0x213FB), "its neighbour is still dealt");
        // The index's 224 areas (pointer tables and beta stages), plus the
        // Coin Ship's two, less 3HB18.
        assert_eq!(headers.len(), 225);
    }
}
