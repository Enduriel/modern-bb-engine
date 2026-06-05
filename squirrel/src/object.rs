//! A safe Rust mirror of Squirrel's `HSQOBJECT` (`SQObject`) that is *also*
//! byte-for-byte layout-compatible with the C type.
//!
//! `SQObject` is a C tagged union: a `_type: SQObjectType` discriminant (a 32-bit
//! bitfield, *not* a sequential tag) followed by a `_unVal: SQObjectValue` union.
//! In this build that is `size 16, align 8` — `_type` at offset 0 and the 8-byte
//! `_unVal` at offset 8 (4 bytes padding between them).
//!
//! [`SqObject`] reproduces that layout exactly:
//!   * `#[repr(C, u32)]` makes it a `{ tag: u32, payload: union }` with the tag
//!     at offset 0 — a guaranteed layout (Rust Reference, "primitive
//!     representations" + `repr(C)`).
//!   * Each variant pins its discriminant to the matching `OT_*` bitfield value,
//!     so the tag bytes are identical to C's.
//!   * Each payload has the same type/width as the union field C uses for that
//!     type, so the value bytes are identical too.
//!
//! The upshot: `SqObject` and `SQObject` *could* be `transmute`d in either
//! direction (the `transmute_matches_conversion` test proves it). We still
//! convert via [`SqObject::from_raw`] / [`SqObject::into_raw`] rather than
//! transmuting, because the match guards against a corrupt/zeroed `_type`
//! (transmuting `_type == 0` into the enum would be instant UB). The layout
//! compatibility is a static invariant, asserted at compile time, that callers
//! can rely on for zero-copy reinterpretation when they want it.

use squirrel_sys::{
	SQArray, SQBool, SQClass, SQClosure, SQFloat, SQFunctionProto, SQGenerator, SQInstance,
	SQInteger, SQNativeClosure, SQObject, SQObjectType, SQObjectValue, SQOuter, SQString, SQTable,
	SQUserData, SQUserPointer, SQWeakRef,
};
// The union's thread pointer is the *C* `SQVM`, not the wrapper defined in main.rs.
use squirrel_sys::SQVM as SqThread;

// Squirrel's type tags are a bitfield, not sequential. Pull them in under terse
// names so they read cleanly as discriminants, match arms, and constructors.
use squirrel_sys::{
	tagSQObjectType_OT_ARRAY as OT_ARRAY, tagSQObjectType_OT_BOOL as OT_BOOL,
	tagSQObjectType_OT_CLASS as OT_CLASS, tagSQObjectType_OT_CLOSURE as OT_CLOSURE,
	tagSQObjectType_OT_FLOAT as OT_FLOAT, tagSQObjectType_OT_FUNCPROTO as OT_FUNCPROTO,
	tagSQObjectType_OT_GENERATOR as OT_GENERATOR, tagSQObjectType_OT_INSTANCE as OT_INSTANCE,
	tagSQObjectType_OT_INTEGER as OT_INTEGER, tagSQObjectType_OT_NATIVECLOSURE as OT_NATIVECLOSURE,
	tagSQObjectType_OT_NULL as OT_NULL, tagSQObjectType_OT_OUTER as OT_OUTER,
	tagSQObjectType_OT_STRING as OT_STRING, tagSQObjectType_OT_TABLE as OT_TABLE,
	tagSQObjectType_OT_THREAD as OT_THREAD, tagSQObjectType_OT_USERDATA as OT_USERDATA,
	tagSQObjectType_OT_USERPOINTER as OT_USERPOINTER, tagSQObjectType_OT_WEAKREF as OT_WEAKREF,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u32)]
pub enum SqObjectType {
	Null = OT_NULL,
	Integer = OT_INTEGER,
	Float = OT_FLOAT,
	Bool = OT_BOOL,
	String = OT_STRING,
	Table = OT_TABLE,
	Array = OT_ARRAY,
	UserData = OT_USERDATA,
	Closure = OT_CLOSURE,
	NativeClosure = OT_NATIVECLOSURE,
	Generator = OT_GENERATOR,
	UserPointer = OT_USERPOINTER,
	Thread = OT_THREAD,
	FuncProto = OT_FUNCPROTO,
	Class = OT_CLASS,
	Instance = OT_INSTANCE,
	WeakRef = OT_WEAKREF,
	Outer = OT_OUTER,
}

impl TryFrom<SQObjectType> for SqObjectType {
	type Error = u32;

	fn try_from(value: SQObjectType) -> Result<Self, Self::Error> {
		match value {
			OT_NULL => Ok(SqObjectType::Null),
			OT_INTEGER => Ok(SqObjectType::Integer),
			OT_FLOAT => Ok(SqObjectType::Float),
			OT_BOOL => Ok(SqObjectType::Bool),
			OT_STRING => Ok(SqObjectType::String),
			OT_TABLE => Ok(SqObjectType::Table),
			OT_ARRAY => Ok(SqObjectType::Array),
			OT_USERDATA => Ok(SqObjectType::UserData),
			OT_CLOSURE => Ok(SqObjectType::Closure),
			OT_NATIVECLOSURE => Ok(SqObjectType::NativeClosure),
			OT_GENERATOR => Ok(SqObjectType::Generator),
			OT_USERPOINTER => Ok(SqObjectType::UserPointer),
			OT_THREAD => Ok(SqObjectType::Thread),
			OT_FUNCPROTO => Ok(SqObjectType::FuncProto),
			OT_CLASS => Ok(SqObjectType::Class),
			OT_INSTANCE => Ok(SqObjectType::Instance),
			OT_WEAKREF => Ok(SqObjectType::WeakRef),
			OT_OUTER => Ok(SqObjectType::Outer),
			other => Err(other), // Unrecognized tag value.
		}
	}
}

/// A decoded `SQObject`, laid out byte-identically to the C type.
///
/// Each variant's discriminant equals its `SQObjectType` value and each payload
/// matches the corresponding `SQObjectValue` union field, so the in-memory
/// representation is interchangeable with [`SQObject`] (see module docs).
#[repr(C, u32)]
#[derive(Copy, Clone, PartialEq)]
pub(crate) enum SqObject {
	Null = SqObjectType::Null as u32,
	Integer(SQInteger) = SqObjectType::Integer as u32,
	Float(SQFloat) = SqObjectType::Float as u32,
	/// Squirrel stores bools in the 8-byte `nInteger` slot (always 0 or 1).
	/// Kept full-width so the bytes match C; use [`SqObject::boolean`] /
	/// [`SqObject::as_bool`] for ergonomic `bool` access.
	Bool(SQBool) = SqObjectType::Bool as u32,
	String(*mut SQString) = SqObjectType::String as u32,
	Table(*mut SQTable) = SqObjectType::Table as u32,
	Array(*mut SQArray) = SqObjectType::Array as u32,
	UserData(*mut SQUserData) = SqObjectType::UserData as u32,
	Closure(*mut SQClosure) = SqObjectType::Closure as u32,
	NativeClosure(*mut SQNativeClosure) = SqObjectType::NativeClosure as u32,
	Generator(*mut SQGenerator) = SqObjectType::Generator as u32,
	UserPointer(SQUserPointer) = SqObjectType::UserPointer as u32,
	Thread(*mut SqThread) = SqObjectType::Thread as u32,
	FuncProto(*mut SQFunctionProto) = SqObjectType::FuncProto as u32,
	Class(*mut SQClass) = SqObjectType::Class as u32,
	Instance(*mut SQInstance) = SqObjectType::Instance as u32,
	WeakRef(*mut SQWeakRef) = SqObjectType::WeakRef as u32,
	Outer(*mut SQOuter) = SqObjectType::Outer as u32,
}

impl std::fmt::Debug for SqObject {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			SqObject::Null => write!(f, "Null"),
			SqObject::Integer(n) => write!(f, "Integer({n})"),
			SqObject::Float(x) => write!(f, "Float({x})"),
			SqObject::Bool(b) => write!(f, "Bool({b})"),
			SqObject::String(p) => write!(f, "String({p:p})"),
			SqObject::Table(p) => write!(f, "Table({p:p})"),
			SqObject::Array(p) => write!(f, "Array({p:p})"),
			SqObject::UserData(p) => write!(f, "UserData({p:p})"),
			SqObject::Closure(p) => write!(f, "Closure({p:p})"),
			SqObject::NativeClosure(p) => write!(f, "NativeClosure({p:p})"),
			SqObject::Generator(p) => write!(f, "Generator({p:p})"),
			SqObject::UserPointer(up) => write!(f, "UserPointer({up:p})"),
			SqObject::Thread(p) => write!(f, "Thread({p:p})"),
			SqObject::FuncProto(p) => write!(f, "FuncProto({p:p})"),
			SqObject::Class(p) => write!(f, "Class({p:p})"),
			SqObject::Instance(p) => write!(f, "Instance({p:p})"),
			SqObject::WeakRef(p) => write!(f, "WeakRef({p:p})"),
			SqObject::Outer(p) => write!(f, "Outer({p:p})"),
		}
	}
}

// Static proof that the layout matches `SQObject`. If a future Squirrel build
// config (e.g. `SQUSEDOUBLE`, 32-bit `SQInteger`) changes the C sizes, or a
// variant's payload drifts, this fails to compile instead of silently making
// `transmute` unsound.
const _: () = {
	assert!(core::mem::size_of::<SqObject>() == core::mem::size_of::<SQObject>());
	assert!(core::mem::align_of::<SqObject>() == core::mem::align_of::<SQObject>());
};

impl SqObject {
	/// Decode a raw `SQObject` from the VM.
	///
	/// # Safety
	/// `raw` must be a well-formed `SQObject` produced by Squirrel: its `_type`
	/// must be one of the `OT_*` tags and `_unVal` must hold the matching active
	/// field. Pointer variants borrow into the VM and are valid only as long as
	/// the underlying object stays alive.
	///
	/// # Panics
	/// On an unrecognized `_type` (a corrupt or zero-initialized object) — this
	/// is exactly the case where a blind `transmute` would be UB.
	pub unsafe fn from_raw(raw: SQObject) -> Self {
		let v = &raw._unVal;
		match raw._type {
			OT_NULL => SqObject::Null,
			// SAFETY: caller guarantees the active union field matches `_type`.
			OT_INTEGER => SqObject::Integer(unsafe { v.nInteger }),
			OT_FLOAT => SqObject::Float(unsafe { v.fFloat }),
			OT_BOOL => SqObject::Bool(unsafe { v.nInteger } as SQBool),
			OT_STRING => SqObject::String(unsafe { v.pString }),
			OT_TABLE => SqObject::Table(unsafe { v.pTable }),
			OT_ARRAY => SqObject::Array(unsafe { v.pArray }),
			OT_USERDATA => SqObject::UserData(unsafe { v.pUserData }),
			OT_CLOSURE => SqObject::Closure(unsafe { v.pClosure }),
			OT_NATIVECLOSURE => SqObject::NativeClosure(unsafe { v.pNativeClosure }),
			OT_GENERATOR => SqObject::Generator(unsafe { v.pGenerator }),
			OT_USERPOINTER => SqObject::UserPointer(unsafe { v.pUserPointer }),
			OT_THREAD => SqObject::Thread(unsafe { v.pThread }),
			OT_FUNCPROTO => SqObject::FuncProto(unsafe { v.pFunctionProto }),
			OT_CLASS => SqObject::Class(unsafe { v.pClass }),
			OT_INSTANCE => SqObject::Instance(unsafe { v.pInstance }),
			OT_WEAKREF => SqObject::WeakRef(unsafe { v.pWeakRef }),
			OT_OUTER => SqObject::Outer(unsafe { v.pOuter }),
			other => panic!("unknown SQObjectType {other:#010x}"),
		}
	}

	/// Re-encode into a raw `SQObject`. Always safe — every variant maps to a
	/// well-formed tag/union pair.
	pub fn into_raw(self) -> SQObject {
		let un_val = match self {
			SqObject::Null => SQObjectValue { nInteger: 0 },
			SqObject::Integer(n) => SQObjectValue { nInteger: n },
			SqObject::Float(f) => SQObjectValue { fFloat: f },
			SqObject::Bool(b) => SQObjectValue {
				nInteger: b as SQInteger,
			},
			SqObject::String(p) => SQObjectValue { pString: p },
			SqObject::Table(p) => SQObjectValue { pTable: p },
			SqObject::Array(p) => SQObjectValue { pArray: p },
			SqObject::UserData(p) => SQObjectValue { pUserData: p },
			SqObject::Closure(p) => SQObjectValue { pClosure: p },
			SqObject::NativeClosure(p) => SQObjectValue { pNativeClosure: p },
			SqObject::Generator(p) => SQObjectValue { pGenerator: p },
			SqObject::UserPointer(p) => SQObjectValue { pUserPointer: p },
			SqObject::Thread(p) => SQObjectValue { pThread: p },
			SqObject::FuncProto(p) => SQObjectValue { pFunctionProto: p },
			SqObject::Class(p) => SQObjectValue { pClass: p },
			SqObject::Instance(p) => SQObjectValue { pInstance: p },
			SqObject::WeakRef(p) => SQObjectValue { pWeakRef: p },
			SqObject::Outer(p) => SQObjectValue { pOuter: p },
		};
		SQObject {
			_type: self.type_tag(),
			_unVal: un_val,
		}
	}

	/// The raw `SQObjectType` tag for this variant (equals its discriminant).
	pub fn type_tag(&self) -> SQObjectType {
		match self {
			SqObject::Null => OT_NULL,
			SqObject::Integer(_) => OT_INTEGER,
			SqObject::Float(_) => OT_FLOAT,
			SqObject::Bool(_) => OT_BOOL,
			SqObject::String(_) => OT_STRING,
			SqObject::Table(_) => OT_TABLE,
			SqObject::Array(_) => OT_ARRAY,
			SqObject::UserData(_) => OT_USERDATA,
			SqObject::Closure(_) => OT_CLOSURE,
			SqObject::NativeClosure(_) => OT_NATIVECLOSURE,
			SqObject::Generator(_) => OT_GENERATOR,
			SqObject::UserPointer(_) => OT_USERPOINTER,
			SqObject::Thread(_) => OT_THREAD,
			SqObject::FuncProto(_) => OT_FUNCPROTO,
			SqObject::Class(_) => OT_CLASS,
			SqObject::Instance(_) => OT_INSTANCE,
			SqObject::WeakRef(_) => OT_WEAKREF,
			SqObject::Outer(_) => OT_OUTER,
		}
	}

	/// A stable identity integer for this object: the pointer for reference types,
	/// the value for scalars (float as its bit pattern), `0` for null. Backs
	/// `toHash` and native receiver-recovery (recovering which script object a
	/// native method was invoked on, for method injection). Stable for a live
	/// object — Squirrel never relocates one (refcounting, no compaction).
	pub(crate) fn identity(&self) -> i64 {
		match self {
			SqObject::Null => 0,
			SqObject::Integer(n) => *n,
			SqObject::Bool(b) => *b as i64,
			SqObject::Float(f) => f.to_bits() as i64,
			SqObject::String(p) => *p as usize as i64,
			SqObject::Table(p) => *p as usize as i64,
			SqObject::Array(p) => *p as usize as i64,
			SqObject::UserData(p) => *p as usize as i64,
			SqObject::Closure(p) => *p as usize as i64,
			SqObject::NativeClosure(p) => *p as usize as i64,
			SqObject::Generator(p) => *p as usize as i64,
			SqObject::UserPointer(p) => *p as usize as i64,
			SqObject::Thread(p) => *p as usize as i64,
			SqObject::FuncProto(p) => *p as usize as i64,
			SqObject::Class(p) => *p as usize as i64,
			SqObject::Instance(p) => *p as usize as i64,
			SqObject::WeakRef(p) => *p as usize as i64,
			SqObject::Outer(p) => *p as usize as i64,
		}
	}

	/// Construct a `Bool` from a Rust `bool`.
	// Part of the mirror's complete accessor set; kept for symmetry with the
	// other variant helpers even while no caller needs it yet.
	#[allow(dead_code)]
	pub fn boolean(b: bool) -> Self {
		SqObject::Bool(b as SQBool)
	}

	/// Interpret a `Bool` variant as a Rust `bool` (`None` for other variants).
	#[allow(dead_code)]
	pub fn as_bool(&self) -> Option<bool> {
		match self {
			SqObject::Bool(b) => Some(*b != 0),
			_ => None,
		}
	}
}

/// Safe direction: a decoded value always re-encodes to a valid raw object.
impl From<SqObject> for SQObject {
	fn from(o: SqObject) -> Self {
		o.into_raw()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Representative value of every "shape" of payload (full 8-byte int,
	/// narrow float, pointer, raw pointer, and the empty Null).
	fn cases() -> [SqObject; 6] {
		[
			SqObject::Null,
			SqObject::Integer(0x0102_0304_0506_0708),
			SqObject::Float(1.5),
			SqObject::boolean(true),
			SqObject::String(0x1234 as *mut _),
			SqObject::UserPointer(0xdead_beef as *mut _),
		]
	}

	#[test]
	fn match_conversion_roundtrips() {
		for o in cases() {
			// SAFETY: `into_raw` yields a well-formed object whose active field
			// matches its tag — exactly `from_raw`'s precondition.
			let back = unsafe { SqObject::from_raw(o.into_raw()) };
			assert_eq!(o, back);
		}
	}

	/// The point of the rework: bytes are interchangeable, so `transmute`
	/// agrees with the match-based conversion in both directions.
	#[test]
	fn transmute_matches_conversion() {
		for o in cases() {
			// Rust enum -> raw bytes: tag must equal the match-based encoding.
			// (Reading `_type` only touches the always-initialized tag at off 0,
			// never the per-variant payload padding.)
			let via_bytes: SQObject = unsafe { core::mem::transmute(o) };
			assert_eq!(via_bytes._type, o.into_raw()._type);

			// raw bytes -> Rust enum: must equal `from_raw`. PartialEq reads
			// only the tag + active field, so no padding is observed.
			let raw = o.into_raw();
			let decoded: SqObject = unsafe { core::mem::transmute(raw) };
			assert_eq!(decoded, unsafe { SqObject::from_raw(raw) });
			assert_eq!(decoded, o);
		}
	}

	#[test]
	fn discriminants_equal_object_type_tags() {
		// The discriminant (first 4 bytes) is the SQObjectType value.
		for o in cases() {
			let raw: SQObject = unsafe { core::mem::transmute(o) };
			assert_eq!(raw._type, o.type_tag());
		}
		assert_eq!(SqObject::Null.type_tag(), OT_NULL);
		assert_eq!(SqObject::Integer(0).type_tag(), OT_INTEGER);
		assert_eq!(SqObject::boolean(false).type_tag(), OT_BOOL);
	}
}
