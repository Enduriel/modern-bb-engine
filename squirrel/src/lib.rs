#![feature(c_variadic)]

mod class;
mod consts;
mod function;
mod identity;
mod object;
mod profile;
mod script_hash;
mod types;
mod vm;

pub use class::{ClassBuilder, Native, NewInstance, SqClass};
pub(crate) use consts::{SQ_ERROR, SQ_OK};
pub(crate) use function::CallArgs;
pub use function::{
	DynNumber, ForwardArgs, Forwarded, MemberKey, Number, Retained, SqStrongRef, StackArg, StackArgs, Stringified, This, Truthy,
	Variadic,
};
pub use identity::{Identity, IdentityHasher, IdentityMap};
pub(crate) use object::SqObject;
pub use object::SqObjectType;
pub use script_hash::ScriptHash;
pub use profile::{CallNode, FunctionKind, Profile, ProfiledFunction, ScriptProfiler};
pub use types::{SqArray, SqBytes, SqRef, SqTable, SqText};
pub use vm::{CensusTrack, CompileError, ErrorValue, Frame, INTERRUPTED, Interrupter, Local, RuntimeError, SQVM};

/// A hash of the vendored VM's sources and build options. It changes whenever the VM or
/// its compiler does, so caches of bytecode can key on it.
pub const VM_SOURCE_HASH: &str = squirrel_sys::SQUIRREL_SOURCE_HASH;

// `SqValue` is the stack-conversion trait for values/elements. It stays
// crate-internal (like `SqArg`/`IntoSqReturn`): it appears only as a bound on the
// public generic methods (`get::<T>`, `call_ret::<T>`, …), so consumers use
// concrete types without ever touching the stack-level `read`/`push_value`.
pub(crate) use types::SqValue;
