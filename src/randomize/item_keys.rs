//! The key vocabulary the model and the ROM share.
//!
//! A **key** is an item the player either holds or does not, which decides
//! whether some gate lets them past. One enum, named the same on both sides,
//! so "what a gate demands" and "what a source hands over" cannot drift into
//! two spellings of the same item.
//!
//! Keys are **never consumed**. "Can the player cross the water" is simply "is
//! there an Anchor in the inventory", which is why the canoe gate carries no
//! SRAM, no completion bit and no per-world state — and why the solver can
//! treat a found key as permanent.

/// An item that opens something.
///
/// The extension point for the next gate: add a variant, give it its inventory
/// byte, and [`crate::randomize::maze::Gate`] can demand it without any other
/// change. The power-up keys (Mushroom/Flower/Leaf/Star) live on
/// `refactor/items-before-pickup` and land here when MiMaze does.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Key {
    /// The canoe key. **Not a `?` block key** — no block dispenses an Anchor.
    /// It is found on the map (a Hammer Bro reward, a Princess letter) and
    /// read by the canoe summon, which is why it needs no dispenser row.
    Anchor,
}

impl Key {
    /// This key's **Global Item ID** — the byte the reward tables hold and the
    /// inventory stores. `items.rs` writes exactly this value.
    // Reason: the site list that stamps a key into a reward table is the
    // caller, and it lands with the placement pass.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) const fn item_byte(self) -> u8 {
        match self {
            Key::Anchor => 0x0A,
        }
    }
}
