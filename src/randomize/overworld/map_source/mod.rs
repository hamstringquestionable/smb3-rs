//! Where each world's map comes from.
//!
//! Everything downstream of this module (the catalog, pickup, the builder, the
//! writer) reads a [`WorldMap`] and never asks where it came from. A source
//! decides *where* things sit and what the tiles are; what each pointer-table
//! entry *is* still comes from the catalog reading the ROM, which is why every
//! world keeps its own budgets whichever source ran.
//!
//! Two sources: [`vanilla`], and [`generated`] for worlds the hidden
//! `generated_worlds` option asks for. See `docs/procedural_maps_design.md`.

pub(crate) mod generated;
#[cfg(test)]
mod inventory;
pub(crate) mod vanilla;

use crate::randomize::rom_data::{Grid, Pos};

/// One world's map, as the rest of the overworld pipeline sees it.
#[derive(Clone)]
pub(crate) struct WorldMap {
    /// Every tile, with gaps still closed (drawn as their water or lock art).
    pub(crate) grid: Grid,
    /// Where each pointer-table entry sits, indexed by `entry_idx`. The grid
    /// must show each entry's own tile at its position: the catalog reads the
    /// tile there to recognise the start.
    pub(crate) entry_pos: Vec<Pos>,
    /// Every active map-object sprite.
    pub(crate) sprites: Vec<Pos>,
    /// The Hammer Bro subset of `sprites`.
    pub(crate) hb_sprites: Vec<Pos>,
    /// Walkable cells drawn closed until a fortress opens them, each with the
    /// path tile it opens to.
    pub(crate) gaps: Vec<(Pos, u8)>,
    /// Blanks reachable only by breaking a rock, which connectivity must leave
    /// alone rather than bridge with a pipe.
    pub(crate) hammer_gated: Vec<Pos>,
    /// Path cells no lock may take (a generated river's `$BA` crossings).
    pub(crate) no_locks: Vec<Pos>,
    /// Where a generated world parks its canoe: the water beside its first
    /// dock. The pipeline moves the world's canoe sprite there. `None` keeps
    /// the sprite where the ROM has it.
    pub(crate) canoe: Option<Pos>,
    /// How far right a generated world's camera pans (`Map_Max_PanR`'s unit,
    /// `$10` a screen). `None` keeps the ROM's.
    pub(crate) pan_limit: Option<u8>,
    /// Where a generated world's castle (its airship entry) and HELP bubble
    /// are: the pipeline moves the airship and HELP sprites there. `None`
    /// keeps them where the ROM has them.
    pub(crate) castle: Option<(Pos, Pos)>,
}

impl WorldMap {
    /// The sprites that are not Hammer Bros (army, canoe, piranhas, ...).
    pub(crate) fn non_hb_sprites(&self) -> Vec<Pos> {
        self.sprites.iter().copied().filter(|p| !self.hb_sprites.contains(p)).collect()
    }
}
