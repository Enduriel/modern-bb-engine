//! Forwarding a native call's arguments to another callable without retaining them:
//! [`StackArg`] borrows an argument slot in place ([`StackArgs`] all the trailing ones,
//! without collecting them), [`This::forward_member`] pushes those slots straight to the
//! callee, and [`Forwarded`] returns the callee's result where it already sits on the
//! stack. A dispatcher built this way (e.g. a Sqrat overload
//! dispatcher) takes no `addref`/`release` (each a `RefTable` hash operation) per
//! argument or for the result, unlike collecting `Variadic<Retained>`.

use std::marker::PhantomData;

use super::strong_ref::read_last_error;
use super::{IntoSqReturn, SqArg, SqStrongRef, This};
use crate::{SQ_OK, SQVM};

/// One argument of the current native call, borrowed in place (no `addref`). The `'vm`
/// lifetime ties it to the call, like [`This`]. Collect them with
/// `Variadic<StackArg>` and pass them on with [`This::forward_member`].
#[derive(Clone, Copy)]
pub struct StackArg<'vm> {
	/// Absolute stack index in the native call's frame (stable while the call runs).
	idx: i64,
	_vm: PhantomData<&'vm SQVM>,
}

impl SqArg for StackArg<'_> {
	type Item<'a> = StackArg<'a>;
	fn push_typemask(mask: &mut String) {
		mask.push('.');
	}
	fn from_sq<'a>(_vm: &'a SQVM, idx: i64) -> Result<StackArg<'a>, String> {
		Ok(StackArg {
			idx,
			_vm: PhantomData,
		})
	}
}

/// All the trailing arguments of the current native call, as a range of stack slots: no
/// allocation and no `addref`. The tail parameter of a forwarding dispatcher,
/// `FnMut(&SQVM, This, StackArgs)`, which accepts any number of arguments.
#[derive(Clone, Copy)]
pub struct StackArgs<'vm> {
	/// Absolute stack index of the first argument after `this`.
	start: i64,
	len: usize,
	_vm: PhantomData<&'vm SQVM>,
}

impl StackArgs<'_> {
	pub(crate) fn new(start: i64, len: usize) -> Self {
		StackArgs {
			start,
			len,
			_vm: PhantomData,
		}
	}

	pub fn len(&self) -> usize {
		self.len
	}

	pub fn is_empty(&self) -> bool {
		self.len == 0
	}
}

/// Arguments [`This::forward_member`] can pass on: a slice of [`StackArg`], or
/// [`StackArgs`].
pub trait ForwardArgs {
	#[doc(hidden)]
	fn push_args(&self, vm: &SQVM) -> usize;
}

impl ForwardArgs for &[StackArg<'_>] {
	fn push_args(&self, vm: &SQVM) -> usize {
		for arg in self.iter() {
			unsafe { squirrel_sys::sq_push(vm.as_ptr(), arg.idx) };
		}
		self.len()
	}
}

impl ForwardArgs for StackArgs<'_> {
	fn push_args(&self, vm: &SQVM) -> usize {
		for i in 0..self.len as i64 {
			unsafe { squirrel_sys::sq_push(vm.as_ptr(), self.start + i) };
		}
		self.len
	}
}

/// A member name for [`This::forward_member`]: a `&str`, or a string made once with
/// [`SqStrongRef::string`], which saves hashing and interning the name on every call.
pub trait MemberKey {
	#[doc(hidden)]
	fn push_key(&self, vm: &SQVM);
}

impl MemberKey for &str {
	fn push_key(&self, vm: &SQVM) {
		vm.push_string(self);
	}
}

impl MemberKey for &SqStrongRef {
	fn push_key(&self, vm: &SQVM) {
		self.push(vm);
	}
}

/// The result of [`This::forward_member`], left on top of the stack. Return it
/// directly from the native closure: it hands that value back as the call's result
/// without retaining it. Pushing anything else first would return that instead.
/// (Only `forward_member` can create one.)
pub struct Forwarded(());

impl IntoSqReturn for Forwarded {
	fn push_return(self, _vm: &SQVM) -> Result<i64, String> {
		Ok(1)
	}
}

impl This<'_> {
	/// Look up member `key` on the receiver and call it with the receiver as `this`
	/// and `args` (slots of the current call) as its arguments, leaving its result on
	/// the stack for the caller to return as [`Forwarded`]. Like
	/// [`call_member_forwarding`](This::call_member_forwarding), the member is
	/// re-resolved on the live receiver every call, and `Ok(None)` means there is no
	/// such member.
	pub fn forward_member(
		&self,
		vm: &SQVM,
		key: impl MemberKey,
		args: impl ForwardArgs,
	) -> Result<Option<Forwarded>, String> {
		let ptr = vm.as_ptr();
		unsafe {
			// The receiver occupies absolute stack index 1 for the whole native call.
			squirrel_sys::sq_push(ptr, 1); // [recv]
			key.push_key(vm); // [recv, key]
			if squirrel_sys::sq_get(ptr, -2) != SQ_OK {
				squirrel_sys::sq_pop(ptr, 1); // `sq_get` popped the key -> []
				return Ok(None);
			}
			squirrel_sys::sq_push(ptr, 1); // [recv, member, this]
			let count = args.push_args(vm); // [recv, member, this, args..]
			if squirrel_sys::sq_call(ptr, count as i64 + 1, 1, 1) == SQ_OK {
				// popped this+args, pushed the result -> [recv, member, result]
				squirrel_sys::sq_remove(ptr, -2); // [recv, result]
				squirrel_sys::sq_remove(ptr, -2); // [result]
				Ok(Some(Forwarded(())))
			} else {
				let error = read_last_error(vm);
				squirrel_sys::sq_pop(ptr, 2); // member + recv -> []
				Err(error)
			}
		}
	}
}
