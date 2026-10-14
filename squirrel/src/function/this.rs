//! [`This`]: the borrowed receiver (`this`) of a native call, usable as a
//! leading closure parameter.

use std::marker::PhantomData;

use super::strong_ref::read_last_error;
use crate::{SQ_OK, SQVM, SqObject, SqObjectType, SqStrongRef};

/// The receiver (`this`) of a native call, borrowed for the duration of the call.
///
/// This is the mirror image of [`Variadic`](super::Variadic): where `Variadic`
/// is a *tail-only* parameter, `This` is a *leading-only* one. Write it as the
/// first parameter of a registered closure to access the receiver:
///
/// ```ignore
/// vm.register("kind", |this: This| matches!(this.object_type(), SqObjectType::Table));
/// vm.register("scaled", |this: This, x: i64| x * 2);
/// ```
///
/// The handle is *borrowed* from the call frame (read via `sq_getstackobj`, with
/// no `addref`), so the `'vm` lifetime ties it to the active call: storing a
/// `This` past the call — e.g. stashing it in a captured `RefCell` — is a
/// compile error, which rules out using it after the receiver may have been
/// collected. Adding `This` to a signature does **not** change the param-check
/// schema Squirrel sees (`this` is already counted, and its type-mask slot stays
/// `.`); it only hands the already-present receiver to your closure.
pub struct This<'vm> {
	object: SqObject,
	_vm: PhantomData<&'vm SQVM>,
}

impl<'vm> This<'vm> {
	/// The runtime type of the receiver — `Table` for a root-table free call,
	/// or the table/instance/class the method was invoked on.
	pub fn object_type(&self) -> SqObjectType {
		SqObjectType::try_from(self.object.type_tag())
			.expect("receiver on the stack has a valid Squirrel object type")
	}

	/// The receiver, held (takes an addref): to return `this` from a native, or keep it.
	pub fn to_strong(&self, vm: &SQVM) -> crate::SqStrongRef {
		crate::SqStrongRef::from_stack(vm, 1)
	}

	/// Read the borrowed receiver at stack index 1. Borrowed (no `addref`); valid
	/// only for the call frame, which `'vm` enforces.
	pub(crate) fn extract(vm: &'vm SQVM) -> This<'vm> {
		let mut raw: squirrel_sys::HSQOBJECT = unsafe { std::mem::zeroed() };
		// `this` always occupies index 1 during a native call. We must never panic
		// here: `extract` runs inside the `extern "C"` trampoline, where unwinding
		// across the FFI boundary is UB. So on any unexpected failure — a read
		// error, or a `_type` tag we don't recognize — fall back to a null receiver
		// instead of letting `SqObject::from_raw` panic on a malformed object.
		let read_ok = unsafe { squirrel_sys::sq_getstackobj(vm.as_ptr(), 1, &mut raw) } == SQ_OK;
		let object = if read_ok && SqObjectType::try_from(raw._type).is_ok() {
			// SAFETY: the read succeeded and `_type` is a tag we recognize, so the
			// active union field matches it.
			unsafe { SqObject::from_raw(raw) }
		} else {
			SqObject::Null
		};
		This {
			object,
			_vm: PhantomData,
		}
	}

	/// Resolve a member named `key` on this receiver and call it as a method (with
	/// `this` bound to the receiver), forwarding `args` positionally. The one VM
	/// capability the overloaded-native layer above needs without touching the stack:
	///
	/// * `Ok(None)` — the receiver has no such member. The caller decides the error
	///   (the Sqrat overload dispatcher turns this into the engine's exact
	///   "No overload matching this argument list found").
	/// * `Ok(Some(ret))` — the member was called; `ret` is its return value (which may
	///   itself be null, e.g. a `spawnDetail` overload returning null on a full tile).
	/// * `Err(e)` — the member raised.
	///
	/// The member is re-resolved on the *live* receiver every call (never captured),
	/// so a mod that replaced a `__sqrat_ol_ name_N` slot is honoured.
	pub fn call_member_forwarding(
		&self,
		vm: &SQVM,
		key: &str,
		args: &[SqStrongRef],
	) -> Result<Option<SqStrongRef>, String> {
		let ptr = vm.as_ptr();
		unsafe {
			// The receiver occupies absolute stack index 1 for the whole native call;
			// duplicating it (rather than re-pushing `self.object`) avoids assuming an
			// addref we do not hold and keeps the stack balanced.
			squirrel_sys::sq_push(ptr, 1); // [recv]
			vm.push_string(key); // [recv, key]
			if squirrel_sys::sq_get(ptr, -2) != SQ_OK {
				// `sq_get` popped the key; nothing pushed. Drop the receiver copy.
				squirrel_sys::sq_pop(ptr, 1); // []
				return Ok(None);
			}
			// [recv, member]. `sq_call` wants [closure, this, args..]; this = recv.
			squirrel_sys::sq_push(ptr, 1); // [recv, member, this]
			let n = args.len() as i64;
			for arg in args {
				arg.push(vm); // [recv, member, this, args..]
			}
			if squirrel_sys::sq_call(ptr, n + 1, 1, 1) == SQ_OK {
				// popped this+args, pushed the result -> [recv, member, result]
				let ret = SqStrongRef::from_stack(vm, -1);
				squirrel_sys::sq_pop(ptr, 3); // result + member + recv -> []
				Ok(Some(ret))
			} else {
				let error = read_last_error(vm);
				squirrel_sys::sq_pop(ptr, 2); // member + recv -> []
				Err(error)
			}
		}
	}
}
