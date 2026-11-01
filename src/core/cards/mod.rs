//! Flashcards: written in notes, scheduled by FSRS. See `parse` for the syntax, `store` for what is remembered.
pub mod fsrs;
pub mod parse;
pub mod store;

pub use parse::{CardKind, RawCard};
pub use store::*;
