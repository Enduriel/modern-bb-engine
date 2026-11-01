//! The roster id argument of `World.getRoster`/`createRoster`/`deleteRoster`.
//!
//! The original's bindings read it as an integer without checking: a value that isn't
//! a number reads as whatever the stack slot held, the VM's address (disassembled,
//! `0x4f8a00`), so every such id names the same roster (`createRoster("foo")` and
//! `getRoster("bar")` are one roster). We raise an error instead, so a mod relying on
//! it learns its id is wrong (an intentional divergence).

use squirrel::{Retained, SQVM, SqObjectType, SqRef};

/// The id a roster native was passed: an integer, or a float read as one (truncated,
/// as Sqrat's integer read does). Anything else is an error naming `native`.
pub(super) fn roster_id(vm: &SQVM, native: &str, id: &Retained) -> Result<i64, String> {
	match id.0.get_value::<SqRef>(vm) {
		Some(SqRef::Integer(n)) => Ok(n),
		Some(SqRef::Float(f)) => Ok(f as i64),
		other => Err(format!(
			"{native}: the roster id must be an integer (got {}); a mod can make one from a name with ::toHash(\"MyModRoster\")",
			type_name(other.as_ref())
		)),
	}
}

/// The value's type as Squirrel's `typeof` names it.
fn type_name(value: Option<&SqRef>) -> &'static str {
	match value {
		None | Some(SqRef::Null) => "null",
		Some(SqRef::Integer(_)) => "integer",
		Some(SqRef::Float(_)) => "float",
		Some(SqRef::Bool(_)) => "bool",
		Some(SqRef::String(_) | SqRef::Bytes(_)) => "string",
		Some(SqRef::Array(_)) => "array",
		Some(SqRef::Table(_)) => "table",
		Some(SqRef::Other(t)) => match t {
			SqObjectType::Closure | SqObjectType::NativeClosure => "function",
			SqObjectType::Class => "class",
			SqObjectType::Instance => "instance",
			SqObjectType::Generator => "generator",
			SqObjectType::UserData => "userdata",
			SqObjectType::WeakRef => "weakref",
			SqObjectType::Thread => "thread",
			_ => "another value",
		},
		Some(_) => "another value",
	}
}
