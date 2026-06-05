//! Safe, ergonomic wrappers for Squirrel's complex (reference) types.
//!
//! Each complex type gets its own module. First up: [`array`] — [`SqArray`], a
//! `Vec`-like handle over a live Squirrel array. [`value::SqValue`] is the shared
//! by-copy element-conversion trait these containers use.
//!
//! Future: `table`, `class`, `instance` (see `FUTURE_EXPANSION.md`).

mod array;
mod deep;
mod dynamic;
mod table;
mod value;

pub use array::SqArray;
pub use dynamic::SqRef;
pub use table::SqTable;
pub use value::SqValue;
