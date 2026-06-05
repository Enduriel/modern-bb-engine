//! [`SqRef`]: a type-matchable, borrowed-handle Squirrel value.

use super::{SqArray, SqTable, SqValue};
use crate::{SQVM, SqObjectType};

/// A type-matchable value read from a Squirrel container.
///
/// Scalars and strings are owned copies; reference types are *safe* borrowed
/// handles (e.g. [`SqArray`]) tied to the VM borrow for `'vm` — never raw
/// pointers. [`SqRef::Other`] carries only the type tag for reference types not
/// yet wrapped; it is read-only for now (pushing it is `unimplemented!`).
///
/// `#[non_exhaustive]`: more variants (tables, classes, instances) will be added,
/// so a `match` on `SqRef` must keep a wildcard arm.
#[non_exhaustive]
#[derive(Debug)]
pub enum SqRef<'vm> {
	Null,
	Integer(i64),
	Float(f64),
	Bool(bool),
	String(String),
	Array(SqArray<'vm>),
	Table(SqTable<'vm>),
	/// A reference type we don't wrap yet (class, instance, closure, …).
	/// Carries only the type tag; cannot be pushed back yet.
	Other(SqObjectType),
}

impl<'vm> SqValue for SqRef<'vm> {
	type Item<'a> = SqRef<'a>;

	fn push_value(self, vm: &SQVM) {
		match self {
			SqRef::Null => vm.push_null(),
			SqRef::Integer(n) => vm.push_integer(n),
			SqRef::Float(f) => vm.push_float(f),
			SqRef::Bool(b) => vm.push_bool(b),
			SqRef::String(s) => vm.push_string(&s),
			SqRef::Array(a) => a.push_value(vm),
			SqRef::Table(t) => t.push_value(vm),
			// Long-term every variant should round-trip; for now `Other` only
			// carries a type tag and has no object to push.
			SqRef::Other(_) => unimplemented!("pushing SqRef::Other is not yet supported"),
		}
	}

	fn read<'a>(vm: &'a SQVM, idx: i64) -> Option<SqRef<'a>> {
		let value = match vm.get_type(idx) {
			SqObjectType::Null => SqRef::Null,
			SqObjectType::Integer => SqRef::Integer(vm.get_integer(idx).ok()?),
			SqObjectType::Float => SqRef::Float(vm.get_float(idx).ok()?),
			SqObjectType::Bool => SqRef::Bool(vm.get_bool(idx).ok()?),
			SqObjectType::String => SqRef::String(vm.get_string(idx).ok()?),
			SqObjectType::Array => SqRef::Array(SqArray::from_stack(vm, idx)?),
			SqObjectType::Table => SqRef::Table(SqTable::from_stack(vm, idx)?),
			other => SqRef::Other(other),
		};
		Some(value)
	}
}
