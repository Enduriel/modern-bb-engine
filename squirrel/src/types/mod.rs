//! Safe, ergonomic wrappers for Squirrel's complex (reference) types.
//!
//! Each complex type gets its own module. First up: [`array`] — [`SqArray`], a
//! `Vec`-like handle over a live Squirrel array. [`value::SqValue`] is the shared
//! by-copy element-conversion trait these containers use.
//!
//! Future: `table`, `class`, `instance` (see `FUTURE_EXPANSION.md`).

mod array;
mod bind;
mod bytes;
mod consts;
mod deep;
mod dynamic;
mod table;
mod text;
mod value;

pub use array::SqArray;
pub use bytes::SqBytes;
pub use dynamic::SqRef;
pub use table::SqTable;
pub use text::SqText;
pub use value::SqValue;
