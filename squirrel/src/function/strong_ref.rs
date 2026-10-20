//! [`SqStrongRef`]: an owned, storable strong reference to a Squirrel object
//! (typically a closure), for callbacks the engine fires on a later call.

use super::{IntoSqReturn, SqArg};
use crate::{SQ_OK, SQVM, SqTable, SqValue};

/// An owned strong reference to a Squirrel object — usually a closure captured as
/// a callback.
///
/// The borrowed handles ([`This`](crate::This), [`SqArray`](crate::SqArray),
/// borrowed `&str` args) are tied to a single native call by their `'vm` lifetime
/// and cannot escape it. `SqStrongRef` is the opposite: it is `'static`, holding
/// an `sq_addref` on the object and releasing it on `Drop`, so the engine can park
/// it in a scheduler / entity / struct and [`call`](SqStrongRef::call) it on a
/// later turn or frame. It keeps the object alive even after the script that
/// created it has returned.
///
/// Capture one straight from a native-function parameter — it implements
/// [`SqArg`] with the closure type mask:
///
/// ```ignore
/// vm.register("schedule", move |cb: SqStrongRef, delay: i64| {
///     scheduler.push(cb, delay);   // stored; fired later via cb.call(&vm, ())
/// });
/// ```
///
/// # Invariant
/// An `SqStrongRef` releases against the VM it was created from. A ref that Rust
/// code outside the VM owns (a host service, a scheduler, a cache) must be dropped
/// before that VM is closed: the engine clears its services first. A ref that the
/// VM's own objects own (a native closure's captures, a native class's slots or
/// payloads) is dropped by `sq_close` itself and then releases nothing, because
/// closing frees every object anyway (see `vm::closing`).
///
/// # Re-entrancy
/// [`call`](SqStrongRef::call) runs arbitrary Squirrel, which may re-enter native
/// functions that borrow engine state. Do not hold a conflicting borrow across a
/// `call`.
///
/// # Suspension
/// The called closure must not suspend (yield / use a generator-style `yield`).
/// A suspended `sq_call` leaves its arguments on the stack instead of popping
/// them, which this code does not account for. Deferred engine callbacks
/// (scheduled events, render, UI) never yield, so this is a non-issue in practice.
pub struct SqStrongRef {
	// Raw handle, used only to release on Drop (and as the VM the ref belongs to).
	vm: squirrel_sys::HSQUIRRELVM,
	// The referenced object, kept alive by an addref taken at construction.
	obj: squirrel_sys::HSQOBJECT,
	// A native instance's payload, once read (null until then): it stays where it is
	// while the instance lives, which this ref ensures (see `SqClass::with_payload`).
	payload: std::cell::Cell<squirrel_sys::SQUserPointer>,
}

impl SqStrongRef {
	/// Capture the object at stack `idx` as a strong reference (takes an addref).
	pub(crate) fn from_stack(vm: &SQVM, idx: i64) -> Self {
		let mut obj: squirrel_sys::HSQOBJECT = unsafe { std::mem::zeroed() };
		unsafe {
			squirrel_sys::sq_getstackobj(vm.as_ptr(), idx, &mut obj);
			squirrel_sys::sq_addref(vm.as_ptr(), &mut obj);
		}
		SqStrongRef {
			vm: vm.as_ptr(),
			obj,
			payload: std::cell::Cell::new(std::ptr::null_mut()),
		}
	}

	/// A reference belongs to the VM it was captured from; using it against
	/// another VM would push an object that VM does not own. The whole engine runs
	/// one VM, so this is a debug-only guard against accidental misuse.
	#[inline]
	fn debug_check_vm(&self, vm: &SQVM) {
		debug_assert_eq!(
			vm.as_ptr(),
			self.vm,
			"SqStrongRef used with a different VM than it was created from"
		);
	}

	/// Push this reference's object onto the stack without consuming `self` (the
	/// stack takes its own reference). Crate-internal building block — e.g. the
	/// native-class factory pushes a cached class before `sq_createinstance`.
	/// A string object, held. Pass it as a [`MemberKey`](crate::MemberKey) to look a
	/// member up without hashing and interning its name each time.
	pub fn string(vm: &SQVM, value: &str) -> Self {
		vm.push_string(value);
		let held = Self::from_stack(vm, -1);
		unsafe { squirrel_sys::sq_pop(vm.as_ptr(), 1) };
		held
	}

	/// A bool, held: for a value an object-valued accessor reads.
	pub fn bool(vm: &SQVM, value: bool) -> Self {
		unsafe { squirrel_sys::sq_pushbool(vm.as_ptr(), value as squirrel_sys::SQBool) };
		let held = Self::from_stack(vm, -1);
		unsafe { squirrel_sys::sq_pop(vm.as_ptr(), 1) };
		held
	}

	/// A new, empty userdata object, held: an opaque per-object token scripts can see
	/// but not use.
	pub fn userdata(vm: &SQVM) -> Self {
		unsafe { squirrel_sys::sq_newuserdata(vm.as_ptr(), 0) };
		let held = Self::from_stack(vm, -1);
		unsafe { squirrel_sys::sq_pop(vm.as_ptr(), 1) };
		held
	}

	/// The native payload [`SqClass::with_payload`](crate::SqClass::with_payload) read
	/// last through this ref (null if none yet).
	pub(crate) fn cached_payload(&self) -> squirrel_sys::SQUserPointer {
		self.payload.get()
	}

	pub(crate) fn cache_payload(&self, payload: squirrel_sys::SQUserPointer) {
		self.payload.set(payload);
	}

	pub(crate) fn push(&self, vm: &SQVM) {
		self.debug_check_vm(vm);
		unsafe { squirrel_sys::sq_pushobject(vm.as_ptr(), self.obj) };
	}

	/// The stable pointer-identity of the referenced object — matches
	/// [`SQVM::this_identity`](crate::SQVM::this_identity) when this same object is
	/// the receiver of a native call. Lets a host registry key objects by identity
	/// (e.g. the state stack recovering which state a method was invoked on).
	pub fn identity(&self) -> i64 {
		// SAFETY: `self.obj` is a well-formed object the ref holds an addref on.
		unsafe { crate::SqObject::from_raw(self.obj) }.identity()
	}

	/// Materialize a borrowed [`SqTable`] handle for this reference's object, or
	/// `None` if it is not a table. The handle takes its own addref, so it is
	/// independent of `self` (and lives for the `vm` borrow). `vm` must be the VM
	/// this reference belongs to.
	pub fn as_table<'vm>(&self, vm: &'vm SQVM) -> Option<SqTable<'vm>> {
		self.debug_check_vm(vm);
		let ptr = vm.as_ptr();
		unsafe { squirrel_sys::sq_pushobject(ptr, self.obj) };
		let table = SqTable::from_stack(vm, -1);
		unsafe { squirrel_sys::sq_pop(ptr, 1) };
		table
	}

	/// Like [`as_table`](Self::as_table), for an array.
	pub fn as_array<'vm>(&self, vm: &'vm SQVM) -> Option<crate::SqArray<'vm>> {
		self.debug_check_vm(vm);
		let ptr = vm.as_ptr();
		unsafe { squirrel_sys::sq_pushobject(ptr, self.obj) };
		let array = crate::SqArray::from_stack(vm, -1);
		unsafe { squirrel_sys::sq_pop(ptr, 1) };
		array
	}

	/// A weak reference to the referenced object (the script's `weakref()`): it doesn't
	/// keep the object alive; [`weak_is_gone`](Self::weak_is_gone) tells when the object
	/// is. A value that isn't reference-counted (a number, null) is its own weak ref.
	pub fn downgrade(&self, vm: &SQVM) -> SqStrongRef {
		self.debug_check_vm(vm);
		let ptr = vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [obj]
			squirrel_sys::sq_weakref(ptr, -1); // [obj, weak]
			let weak = SqStrongRef::from_stack(vm, -1);
			squirrel_sys::sq_pop(ptr, 2);
			weak
		}
	}

	/// Whether this is a weak reference whose object is gone (or a null). False for
	/// any other value.
	pub fn weak_is_gone(&self, vm: &SQVM) -> bool {
		self.debug_check_vm(vm);
		let ptr = vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [weak]
			if squirrel_sys::sq_getweakrefval(ptr, -1) != SQ_OK {
				// Not a weak reference: nothing pushed.
				let null = squirrel_sys::sq_gettype(ptr, -1) == squirrel_sys::tagSQObjectType_OT_NULL;
				squirrel_sys::sq_pop(ptr, 1);
				return null;
			}
			// [weak, value]
			let gone = squirrel_sys::sq_gettype(ptr, -1) == squirrel_sys::tagSQObjectType_OT_NULL;
			squirrel_sys::sq_pop(ptr, 2);
			gone
		}
	}

	/// Call the referenced closure, discarding any return value.
	///
	/// Arguments are passed as a tuple (`()`, `(a,)`, `(a, b)`, …). `this` is the
	/// root table; a closure bound with `bindenv` uses its bound environment
	/// instead. Returns the thrown error text if the closure raises.
	pub fn call<A: CallArgs>(&self, vm: &SQVM, args: A) -> Result<(), String> {
		self.debug_check_vm(vm);
		let ptr = vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [closure]
			squirrel_sys::sq_pushroottable(ptr); // [closure, this]
			let n = args.push_all(vm); // [closure, this, args..]
			if squirrel_sys::sq_call(ptr, n + 1, 0, 1) == SQ_OK {
				// sq_call popped `this` + args; the closure stays -> [closure]
				squirrel_sys::sq_pop(ptr, 1);
				Ok(())
			} else {
				// sq_call popped `this` + args; the closure stays -> [closure]
				let error = read_last_error(vm);
				squirrel_sys::sq_pop(ptr, 1);
				Err(error)
			}
		}
	}

	/// Call the referenced closure with `this` bound to `this_obj`, discarding any
	/// return value (e.g. running a script file in its own scope table). Returns the
	/// thrown error text if the closure raises.
	pub fn call_with_this<A: CallArgs>(&self, vm: &SQVM, this_obj: &SqStrongRef, args: A) -> Result<(), String> {
		self.debug_check_vm(vm);
		let ptr = vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [closure]
			squirrel_sys::sq_pushobject(ptr, this_obj.obj); // [closure, this]
			let n = args.push_all(vm); // [closure, this, args..]
			if squirrel_sys::sq_call(ptr, n + 1, 0, 1) == SQ_OK {
				squirrel_sys::sq_pop(ptr, 1);
				Ok(())
			} else {
				let error = read_last_error(vm);
				squirrel_sys::sq_pop(ptr, 1);
				Err(error)
			}
		}
	}

	/// Call a method `method` on this reference's object (a table/instance) with
	/// `this` bound to the object itself — i.e. `obj.method(args)`. Discards the
	/// return value. Errors if `method` is not found or the call raises.
	///
	/// This is how the engine drives a script-side method from Rust (e.g. the UI
	/// harness firing `screen.onOkPressed(...)` exactly as the JS dispatcher would).
	pub fn call_method<A: CallArgs>(&self, vm: &SQVM, method: &str, args: A) -> Result<(), String> {
		self.debug_check_vm(vm);
		let ptr = vm.as_ptr();
		unsafe {
			self.push(vm); // [obj]
			vm.push_string(method); // [obj, "method"]
			if squirrel_sys::sq_get(ptr, -2) != SQ_OK {
				// `sq_get` popped the key; nothing pushed. Drop the object.
				squirrel_sys::sq_pop(ptr, 1); // []
				return Err(format!("no method '{method}' on the object"));
			}
			// [obj, closure]. sq_call wants [closure, this, args..]; push this=obj.
			self.push(vm); // [obj, closure, this]
			let n = args.push_all(vm); // [obj, closure, this, args..]
			if squirrel_sys::sq_call(ptr, n + 1, 0, 1) == SQ_OK {
				// popped this+args; closure stays -> [obj, closure]
				squirrel_sys::sq_pop(ptr, 2); // pop closure + obj -> []
				Ok(())
			} else {
				let error = read_last_error(vm);
				squirrel_sys::sq_pop(ptr, 2);
				Err(error)
			}
		}
	}

	/// Like [`call_method`](Self::call_method) but reads the method's return value
	/// as `R` (`None` if it is not an `R`) — for the synchronous query path where a
	/// script method computes a value the caller needs (e.g. `onQuery*TooltipData`).
	pub fn call_method_ret<'a, R: SqValue, A: CallArgs>(
		&self,
		vm: &'a SQVM,
		method: &str,
		args: A,
	) -> Result<Option<R::Item<'a>>, String> {
		self.debug_check_vm(vm);
		let ptr = vm.as_ptr();
		unsafe {
			self.push(vm); // [obj]
			vm.push_string(method); // [obj, "method"]
			if squirrel_sys::sq_get(ptr, -2) != SQ_OK {
				squirrel_sys::sq_pop(ptr, 1); // []
				return Err(format!("no method '{method}' on the object"));
			}
			self.push(vm); // [obj, closure, this]
			let n = args.push_all(vm); // [obj, closure, this, args..]
			if squirrel_sys::sq_call(ptr, n + 1, 1, 1) == SQ_OK {
				// popped this+args, pushed result -> [obj, closure, result]
				let value = R::read(vm, -1);
				squirrel_sys::sq_pop(ptr, 3); // result + closure + obj -> []
				Ok(value)
			} else {
				let error = read_last_error(vm);
				squirrel_sys::sq_pop(ptr, 2); // closure + obj
				Err(error)
			}
		}
	}

	/// Read the referenced value itself as `R` (e.g. a `Number` held in a variadic
	/// argument). `None` if it is not an `R`.
	pub fn get_value<'a, R: SqValue>(&self, vm: &'a SQVM) -> Option<R::Item<'a>> {
		self.push(vm); // [value]
		let value = R::read(vm, -1);
		unsafe { squirrel_sys::sq_pop(vm.as_ptr(), 1) }; // []
		value
	}

	/// Read member `key` off this object as `R` — the read-only counterpart to
	/// [`call_method`](Self::call_method). Works on tables AND native instances
	/// (`sq_get` triggers the instance's `_get` metamethod, so a native property like
	/// `tile.X` is read too). `None` if the member is absent or not an `R`.
	pub fn get_member<'a, R: SqValue>(&self, vm: &'a SQVM, key: &str) -> Option<R::Item<'a>> {
		let ptr = vm.as_ptr();
		unsafe {
			self.push(vm); // [obj]
			vm.push_string(key); // [obj, key]
			if squirrel_sys::sq_get(ptr, -2) != SQ_OK {
				// `sq_get` consumed the key; nothing pushed. Drop the object.
				squirrel_sys::sq_pop(ptr, 1); // []
				return None;
			}
			// [obj, value]
			let value = R::read(vm, -1);
			squirrel_sys::sq_pop(ptr, 2); // value + obj -> []
			value
		}
	}

	/// Assign member `key` on this object (`obj.key = value`): the write counterpart to
	/// [`get_member`](Self::get_member). Uses `sq_set`, so a native instance's `_set`
	/// metamethod runs (e.g. `tile.IsOccupied = true`); like a script assignment it does
	/// not create a missing table slot.
	pub fn set_member<V: SqValue>(&self, vm: &SQVM, key: &str, value: V) -> Result<(), String> {
		let ptr = vm.as_ptr();
		unsafe {
			self.push(vm); // [obj]
			vm.push_string(key); // [obj, key]
			value.push_value(vm); // [obj, key, value]
			let ok = squirrel_sys::sq_set(ptr, -3) == SQ_OK; // pops key + value -> [obj]
			squirrel_sys::sq_pop(ptr, 1);
			if ok {
				Ok(())
			} else {
				let message = read_last_error(vm);
				squirrel_sys::sq_reseterror(ptr);
				Err(format!("set '{key}': {message}"))
			}
		}
	}

	/// Call the referenced closure and read its return value as `R`, or `None` if
	/// the return value is not an `R`.
	pub fn call_ret<'a, R: SqValue, A: CallArgs>(
		&self,
		vm: &'a SQVM,
		args: A,
	) -> Result<Option<R::Item<'a>>, String> {
		self.debug_check_vm(vm);
		let ptr = vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [closure]
			squirrel_sys::sq_pushroottable(ptr); // [closure, this]
			let n = args.push_all(vm); // [closure, this, args..]
			if squirrel_sys::sq_call(ptr, n + 1, 1, 1) == SQ_OK {
				// sq_call popped `this` + args and pushed the result -> [closure, result]
				let value = R::read(vm, -1);
				squirrel_sys::sq_pop(ptr, 2);
				Ok(value)
			} else {
				// sq_call popped `this` + args; the closure stays -> [closure]
				let error = read_last_error(vm);
				squirrel_sys::sq_pop(ptr, 1);
				Err(error)
			}
		}
	}
}

impl Clone for SqStrongRef {
	/// Another owning reference to the same object (takes a fresh addref).
	fn clone(&self) -> Self {
		let mut obj = self.obj;
		unsafe { squirrel_sys::sq_addref(self.vm, &mut obj) };
		SqStrongRef {
			vm: self.vm,
			obj,
			payload: self.payload.clone(),
		}
	}
}

impl Drop for SqStrongRef {
	fn drop(&mut self) {
		// A ref the VM's own objects hold drops while `sq_close` frees them, after
		// the VM it would release against is gone (see `vm::closing`).
		if crate::vm::closing::is_closing(self.vm) {
			return;
		}
		unsafe { squirrel_sys::sq_release(self.vm, &mut self.obj) };
	}
}

/// Capture a callback parameter as a strong reference. The type mask is `c`
/// (closure / native closure), so Squirrel rejects non-callable arguments.
impl SqArg for SqStrongRef {
	type Item<'a> = SqStrongRef;
	fn push_typemask(mask: &mut String) {
		mask.push('c');
	}
	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<SqStrongRef, String> {
		Ok(SqStrongRef::from_stack(vm, idx))
	}
}

/// Returning a strong ref hands its object back to Squirrel — the vehicle for a
/// native function that builds a table/array and returns it (see
/// [`SqTable::to_strong`](crate::SqTable::to_strong)). A borrowed handle cannot
/// be a return type (its lifetime is the call), but a `SqStrongRef` is owned.
impl IntoSqReturn for SqStrongRef {
	fn push_return(self, vm: &SQVM) -> Result<i64, String> {
		// Push the object; `self` then drops (releasing our addref), but the stack
		// now holds a counted reference for Squirrel to take as the return value.
		unsafe { squirrel_sys::sq_pushobject(vm.as_ptr(), self.obj) };
		Ok(1)
	}
}

/// An `SqStrongRef` is itself a value: pushed by reference, read back as a fresh
/// strong reference. This lets one be stored into a table/array slot — e.g. a
/// native function built with [`SQVM::make_closure`](crate::SQVM::make_closure)
/// placed onto a sub-table like `IO` or `Math`.
impl SqValue for SqStrongRef {
	type Item<'a> = SqStrongRef;
	fn push_value(self, vm: &SQVM) {
		// The stack takes its own reference; `self` then drops, releasing ours.
		unsafe { squirrel_sys::sq_pushobject(vm.as_ptr(), self.obj) };
	}
	fn read<'a>(vm: &'a SQVM, idx: i64) -> Option<SqStrongRef> {
		Some(SqStrongRef::from_stack(vm, idx))
	}
}

/// Captures a native-call argument of *any* type as an owned [`SqStrongRef`], so
/// it can be retained past the call. Type mask `.` (any).
///
/// Unlike a bare `SqStrongRef` parameter (mask `c`, closure-only), `Retained`
/// accepts anything — a table, instance, scalar, or `null`. It is the companion to
/// a closure parameter for "callback + arbitrary userdata" APIs like
/// `Time.scheduleEvent(unit, delay, callback, userdata)`.
pub struct Retained(pub SqStrongRef);

impl SqArg for Retained {
	type Item<'a> = Retained;
	fn push_typemask(mask: &mut String) {
		mask.push('.');
	}
	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<Retained, String> {
		Ok(Retained(SqStrongRef::from_stack(vm, idx)))
	}
}

/// Read and clear the VM's last error as a `String` (best effort).
pub(crate) fn read_last_error(vm: &SQVM) -> String {
	let ptr = vm.as_ptr();
	unsafe {
		squirrel_sys::sq_getlasterror(ptr); // [.., error]
		let message = vm
			.get_str_ref(-1)
			.map(str::to_owned)
			.unwrap_or_else(|_| "squirrel call raised a non-string error".to_string());
		squirrel_sys::sq_pop(ptr, 1);
		squirrel_sys::sq_reseterror(ptr);
		message
	}
}

/// Arguments passed to [`SqStrongRef::call`], as a tuple of [`SqValue`]s. Pushes
/// each onto the stack and reports how many.
pub trait CallArgs {
	fn push_all(self, vm: &SQVM) -> i64;
}

impl CallArgs for () {
	fn push_all(self, _vm: &SQVM) -> i64 {
		0
	}
}

impl<A: SqValue> CallArgs for (A,) {
	fn push_all(self, vm: &SQVM) -> i64 {
		self.0.push_value(vm);
		1
	}
}

impl<A: SqValue, B: SqValue> CallArgs for (A, B) {
	fn push_all(self, vm: &SQVM) -> i64 {
		self.0.push_value(vm);
		self.1.push_value(vm);
		2
	}
}

impl<A: SqValue, B: SqValue, C: SqValue> CallArgs for (A, B, C) {
	fn push_all(self, vm: &SQVM) -> i64 {
		self.0.push_value(vm);
		self.1.push_value(vm);
		self.2.push_value(vm);
		3
	}
}

impl<A: SqValue, B: SqValue, C: SqValue, D: SqValue> CallArgs for (A, B, C, D) {
	fn push_all(self, vm: &SQVM) -> i64 {
		self.0.push_value(vm);
		self.1.push_value(vm);
		self.2.push_value(vm);
		self.3.push_value(vm);
		4
	}
}

#[cfg(test)]
mod tests {
	use std::cell::RefCell;
	use std::rc::Rc;

	use super::SqStrongRef;
	use crate::SQVM;

	#[test]
	fn captures_and_fires_a_callback_after_the_script_returns() {
		let mut vm = SQVM::new(1024);

		// A native fn that stashes a callback into Rust, and one the callback calls.
		let stash: Rc<RefCell<Option<SqStrongRef>>> = Rc::new(RefCell::new(None));
		let stash_in = stash.clone();
		vm.register("remember", move |cb: SqStrongRef| {
			*stash_in.borrow_mut() = Some(cb);
		});

		let log: Rc<RefCell<Vec<i64>>> = Rc::new(RefCell::new(Vec::new()));
		let log_in = log.clone();
		vm.register("record", move |n: i64| {
			log_in.borrow_mut().push(n);
		});

		vm.execute_squirrel_script(r#"remember(function () { record(42); });"#)
			.expect("script registers the callback");

		// The script has returned; only our strong ref keeps the closure alive.
		let cb = stash.borrow_mut().take().expect("callback captured");
		cb.call(&vm, ()).expect("callback runs");
		assert_eq!(*log.borrow(), vec![42]);

		// Fire it again to confirm it stays valid.
		cb.call(&vm, ()).expect("callback runs again");
		assert_eq!(*log.borrow(), vec![42, 42]);
	}

	#[test]
	fn passes_args_and_reads_return_value() {
		let mut vm = SQVM::new(1024);

		let stash: Rc<RefCell<Option<SqStrongRef>>> = Rc::new(RefCell::new(None));
		let stash_in = stash.clone();
		vm.register("remember", move |cb: SqStrongRef| {
			*stash_in.borrow_mut() = Some(cb);
		});

		vm.execute_squirrel_script(r#"remember(function (a, b) { return a * b; });"#)
			.expect("script registers the callback");

		let cb = stash.borrow_mut().take().expect("callback captured");
		let result = cb
			.call_ret::<i64, _>(&vm, (6i64, 7i64))
			.expect("callback runs");
		assert_eq!(result, Some(42));
	}

	#[test]
	fn raised_error_is_returned() {
		let mut vm = SQVM::new(1024);

		let stash: Rc<RefCell<Option<SqStrongRef>>> = Rc::new(RefCell::new(None));
		let stash_in = stash.clone();
		vm.register("remember", move |cb: SqStrongRef| {
			*stash_in.borrow_mut() = Some(cb);
		});

		vm.execute_squirrel_script(r#"remember(function () { throw "boom"; });"#)
			.expect("script registers the callback");

		let cb = stash.borrow_mut().take().expect("callback captured");
		let err = cb.call(&vm, ()).expect_err("callback throws");
		assert!(
			err.contains("boom"),
			"error text should carry the thrown value: {err}"
		);

		// The VM is still usable afterwards (error was consumed).
		vm.execute_squirrel_script(r#"::print("");"#)
			.expect("vm still usable after a handled callback error");
	}

	#[test]
	fn rejects_non_closure_argument() {
		let mut vm = SQVM::new(1024);
		vm.register("remember", move |_cb: SqStrongRef| {});
		// The `c` type mask makes Squirrel reject a non-callable argument.
		vm.execute_squirrel_script(r#"remember(42);"#)
			.expect_err("an integer is not a closure");
	}
}
