//! Registering Rust-backed **native classes** with the Squirrel VM.
//!
//! Squirrel scripts see these as ordinary classes (`type(x) == "instance"`,
//! `x instanceof color`), but each instance is backed by a Rust value `T` rather
//! than plain script members. Fields are exposed as properties whose getter/setter
//! read and write that `T` (via the `_get`/`_set` metamethods), so derived views
//! stay consistent — e.g. a `color`'s `R`/`G`/`B`/`A`/`RGBA` over one `u32`.
//!
//! This is the generic mechanism only; the concrete BB value types (`vec2f`,
//! `vec2i`, `color`, …) are defined on top of it in the `squirrel-bb` crate.
//!
//! Pieces:
//!   * [`payload`]  — the `TypeId`-tagged heap payload + safe read-back
//!   * [`property`] — `FieldValue`, `Property`, and the `_get`/`_set` metamethods
//!   * [`builder`]  — [`ClassBuilder`] / [`SqClass`]: define a class, make instances
//!   * [`arg`]      — [`Native<T>`]: take an instance as a native-fn argument
//!
//! # Limitations
//! * Instances must be created via [`SqClass::instantiate`] (i.e. a factory like
//!   `createVec`). A script-side `clone` or bare-constructor call produces an
//!   instance with no Rust payload; field access on it errors (it is never
//!   silently unsound — `payload_ptr` returns `None`).
//! * Because a class declares no real members, every field read goes through
//!   `_get`, which errors on an unknown key. The instance default-delegate methods
//!   (`tostring`/`weakref`/`getclass`/`rawget`/…) are therefore not reachable by
//!   an explicit `inst.method()` call. `instanceof` and a separately-registered
//!   `_tostring` metamethod are unaffected. (BB's vec/color are only ever read via
//!   their value fields, so this is not exercised in practice.)

mod arg;
mod builder;
mod method;
mod payload;
mod property;

pub use arg::Native;
pub use builder::{ClassBuilder, NewInstance, SqClass};

#[cfg(test)]
mod tests;
