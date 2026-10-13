//! The Rust payload carried by a native-class instance, and safe access to it.
//!
//! Each instance of a native class created through this module stores a boxed
//! [`NativePayload<T>`] as its Squirrel "instance user pointer", with a release
//! hook that drops the box when the instance is collected. The payload begins
//! with a [`TypeId`] header (`#[repr(C)]`, so it is at offset 0 for every `T`),
//! and every native class shares one Squirrel *typetag* ([`native_payload_tag`]).
//! Reading a payload back therefore goes through two checks: the shared typetag
//! proves the instance actually carries a `NativePayload` (so the header is safe
//! to read), and the `TypeId` proves it is the specific `T` expected.

use std::any::TypeId;

use crate::{SQ_OK, SQVM};

/// One process-wide marker; its address is the Squirrel typetag set on every
/// native class. An instance whose typetag matches is known to carry a
/// [`NativePayload`], making its header safe to read.
static NATIVE_PAYLOAD_TAG: u8 = 0;

/// The shared typetag pointer (the address of [`NATIVE_PAYLOAD_TAG`]).
pub(crate) fn native_payload_tag() -> squirrel_sys::SQUserPointer {
	core::ptr::addr_of!(NATIVE_PAYLOAD_TAG) as squirrel_sys::SQUserPointer
}

/// The heap block a native instance points at: a type tag followed by the value.
/// `#[repr(C)]` keeps `type_id` at offset 0 regardless of `T`, so it can be read
/// before the concrete `T` is known.
#[repr(C)]
pub(crate) struct NativePayload<T> {
	pub(crate) type_id: TypeId,
	pub(crate) value: T,
}

impl<T: 'static> NativePayload<T> {
	/// Box a value into a raw payload pointer for `sq_setinstanceup` (ownership
	/// transfers to the instance; [`instance_release_hook`] frees it).
	pub(crate) fn into_raw(value: T) -> squirrel_sys::SQUserPointer {
		let boxed = Box::new(NativePayload {
			type_id: TypeId::of::<T>(),
			value,
		});
		Box::into_raw(boxed) as squirrel_sys::SQUserPointer
	}
}

/// Drop the boxed [`NativePayload<T>`] behind a collected instance.
pub(crate) unsafe extern "C" fn instance_release_hook<T>(
	ptr: squirrel_sys::SQUserPointer,
	_size: squirrel_sys::SQInteger,
) -> squirrel_sys::SQInteger {
	if !ptr.is_null() {
		drop(unsafe { Box::from_raw(ptr as *mut NativePayload<T>) });
	}
	0
}

/// Resolve the instance at `idx` to its `NativePayload<T>` pointer, or `None` if
/// it is not a native instance of exactly `T`.
///
/// # Safety
/// Returns a raw pointer into the instance's heap payload, valid for as long as
/// the instance stays alive (it is on the stack during the call that produced
/// `idx`). The caller must not retain it past that.
pub(crate) unsafe fn payload_ptr<T: 'static>(vm: &SQVM, idx: i64) -> Option<*mut NativePayload<T>> {
	let ptr = vm.as_ptr();
	// 1. Typetag must be ours — proves this instance carries a NativePayload, so
	//    reading the header below is sound (not an arbitrary foreign pointer).
	let mut tag: squirrel_sys::SQUserPointer = std::ptr::null_mut();
	if unsafe { squirrel_sys::sq_gettypetag(ptr, idx, &mut tag) } != SQ_OK {
		return None;
	}
	if tag != native_payload_tag() {
		return None;
	}
	// 2. Fetch the payload pointer (no per-call typetag check; we did our own).
	let mut up: squirrel_sys::SQUserPointer = std::ptr::null_mut();
	// `throwerror = false`: a miss is reported as `None`, not as a VM error.
	if unsafe { squirrel_sys::sq_getinstanceup(ptr, idx, &mut up, std::ptr::null_mut(), 0) } != SQ_OK {
		return None;
	}
	if up.is_null() {
		return None;
	}
	// 3. The TypeId header must match the concrete `T`.
	let payload = up as *mut NativePayload<T>;
	if unsafe { (*payload).type_id } != TypeId::of::<T>() {
		return None;
	}
	Some(payload)
}
