//! Curated palette-group variants for themed palette randomization.
//!
//! Each entry is a position in the ROM (file offset) where a 4-byte palette
//! group lives, paired with two or more known-good variants (vanilla + sources
//! like "Super Mario Bros. 3 Recolored v1.0"). At randomization time the
//! randomizer picks one variant per position, so every combination emitted
//! is built from aesthetically pre-validated 4-byte groups — no pool-mixing,
//! no independent-byte picks, no clash risk.
//!
//! This sidesteps the failure mode where flat color-pool randomization
//! produces combinations the original palette artists never intended.
//!
//! Future expansion: add additional curated variants (hand-tuned or from
//! other palette hacks) to each entry's `variants` list.

/// A palette-group variant set at a specific file offset.
pub struct VariantGroup {
    pub offset: usize,
    /// List of known-good 4-byte variants. At least one variant (vanilla)
    /// must always be present. Additional variants widen the randomization
    /// space without adding clash risk.
    pub variants: &'static [[u8; 4]],
}

// ----------------------------------------------------------------------------
// Plains tileset — quartets that Recolored changed relative to vanilla.
// Quartets Recolored left unchanged are omitted (they stay vanilla always).
// Source: tools/extract_recolored_pools.py, dumped 2026.
// ----------------------------------------------------------------------------

/// Slot 3 (0x36BE4 band 3) — plains BG + HUD. 7 positions changed by Recolored.
pub const PLAINS_SLOT3_VARIANTS: &[VariantGroup] = &[
    VariantGroup { offset: 0x36C94, variants: &[
        [0x36, 0x0F, 0xFF, 0x1A],  // vanilla
        [0x37, 0x06, 0xFF, 0x1A],  // recolored
    ]},
    VariantGroup { offset: 0x36CA0, variants: &[
        [0x30, 0x0F, 0x3C, 0x0F],  // vanilla
        [0x30, 0x0F, 0x11, 0x02],  // recolored
    ]},
    VariantGroup { offset: 0x36CA4, variants: &[
        [0x30, 0x3C, 0x3C, 0x0F],  // vanilla
        [0x30, 0x22, 0x3C, 0x18],  // recolored
    ]},
    VariantGroup { offset: 0x36CA8, variants: &[
        [0x36, 0x27, 0x3C, 0x0F],  // vanilla
        [0x38, 0x28, 0x3C, 0x0A],  // recolored
    ]},
    VariantGroup { offset: 0x36CAC, variants: &[
        [0x2A, 0x1A, 0x3C, 0x0F],  // vanilla
        [0x2A, 0x1A, 0x3C, 0x01],  // recolored
    ]},
    VariantGroup { offset: 0x36CB4, variants: &[
        [0x30, 0x3C, 0x36, 0x0F],  // vanilla
        [0x22, 0x32, 0x1A, 0x18],  // recolored
    ]},
    VariantGroup { offset: 0x36CB8, variants: &[
        [0x36, 0x27, 0x36, 0x0F],  // vanilla
        [0x38, 0x28, 0x36, 0x0F],  // recolored
    ]},
    VariantGroup { offset: 0x36CC0, variants: &[
        [0x31, 0x12, 0x37, 0x0F],  // vanilla
        [0x31, 0x12, 0x11, 0x0F],  // recolored
    ]},
];

/// Slice 4 band 3 (0x376D8) — plains BG variant. 1 position changed by Recolored.
pub const PLAINS_SLICE4_VARIANTS: &[VariantGroup] = &[
    VariantGroup { offset: 0x37704, variants: &[
        [0x36, 0x0F, 0xFF, 0x30],  // vanilla
        [0x37, 0x06, 0xFF, 0x30],  // recolored
    ]},
];
