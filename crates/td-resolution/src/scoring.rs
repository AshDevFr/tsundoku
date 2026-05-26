//! Backwards-compat re-export of the canonical Dice scoring helpers.
//!
//! The implementation lives in [`td_metadata::scoring`] so individual
//! providers (notably MangaBaka's offline store) can re-rank their own
//! search results without taking a dependency on `td-resolution`.

pub use td_metadata::scoring::{best_dice, dice};
