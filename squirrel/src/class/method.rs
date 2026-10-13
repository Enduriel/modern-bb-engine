//! Native **methods** on a native class — typed and marshalled exactly like
//! [`register`](crate::SQVM::register), but with the instance's payload injected
//! as a leading `&mut T` receiver.
//!
//! A property ([`property`](super::property)) is a field read/written through
//! `_get`/`_set`. A *method* is a real callable member slotted onto the class, so
//! `inst.doThing(args)` resolves by ordinary member lookup (instance → class)
//! *before* the `_get` fallback ever runs.
//!
//! The body is an ordinary Rust closure: the receiver `&mut T` is resolved from
//! `this`, each argument is read via its [`SqArg`], and the return goes back via
//! [`IntoSqReturn`] — so a method body never touches the Squirrel stack (the same
//! guarantee the typed `register` gives plain functions). Two shapes are accepted,
//! disambiguated by the `Marker`:
//!   * `FnMut(&mut T, args..) -> R` — a plain method;
//!   * `FnMut(&mut T, &SQVM, args..) -> R` — a method that also needs the VM (e.g.
//!     to build a table or instantiate another native object as its return).
//!
//! This backs e.g. `color.varyRGB(...)`, the `TileTransition` setters, and the UI
//! `JSHandle.connectToModule(...)` (which mints a child handle via the VM).

use super::payload::payload_ptr;
use crate::SQVM;
use crate::function::{IntoSqReturn, SqArg, SqFunction, Variadic};

/// The concrete [`SqFunction`] produced for one native method: the marshalling
/// closure plus its precomputed param-check schema (`this` + the fixed args).
struct MethodFn {
	invoke: Box<dyn FnMut(&SQVM) -> squirrel_sys::SQInteger>,
	nparamscheck: i64,
	typemask: String,
}

impl SqFunction for MethodFn {
	fn invoke(&mut self, vm: &SQVM) -> squirrel_sys::SQInteger {
		(self.invoke)(vm)
	}
	fn nparamscheck(&self) -> i64 {
		self.nparamscheck
	}
	fn typemask(&self) -> Option<&str> {
		Some(self.typemask.as_str())
	}
}

/// Distinct marker heads so the plain and VM-receiving impls never overlap;
/// callers never name them.
#[doc(hidden)]
pub struct PlainRecv;
#[doc(hidden)]
pub struct VmRecv;
#[doc(hidden)]
pub struct VariadicRecv;
#[doc(hidden)]
pub struct VmVariadicRecv;

/// Converts a method closure into a boxed [`SqFunction`]. `Marker` (a head marker
/// plus the argument tuple) keeps the per-shape/arity impls from overlapping;
/// callers never name it. Resolves the receiver payload `&mut T` from `this` (stack
/// index 1) before parsing args.
pub trait IntoSqMethod<T, Marker> {
	fn into_method_fn(self, name: String) -> Box<dyn SqFunction>;
}

/// Expands to `1` per argument, for counting fixed parameters.
macro_rules! count_one {
	($t:ident) => {
		1i64
	};
}

/// Shared prologue: resolve the `&mut T` receiver from `this` or throw. Expands to
/// a `match` that `return`s the error sentinel on a wrong receiver.
macro_rules! resolve_receiver {
	($vm:ident, $name:ident, $T:ty) => {
		match unsafe { payload_ptr::<$T>($vm, 1) } {
			Some(payload) => payload,
			None => {
				return $vm.throw_error(&format!(
					"{}: receiver is not an instance of this native class",
					$name
				));
			}
		}
	};
}

macro_rules! impl_into_sq_method {
	($($arg:ident),*) => {
		// -------- plain: FnMut(&mut T, args..) -> R --------
		impl<T, Func, Ret, $($arg),*> IntoSqMethod<T, (PlainRecv, $($arg,)*)> for Func
		where
			T: 'static,
			Func: FnMut(&mut T, $($arg),*) -> Ret
				+ for<'a> FnMut(&mut T, $(<$arg as SqArg>::Item<'a>),*) -> Ret
				+ 'static,
			Ret: IntoSqReturn,
			$($arg: SqArg + 'static,)*
		{
			fn into_method_fn(self, name: String) -> Box<dyn SqFunction> {
				#[allow(unused_mut)]
				let mut typemask = String::from("."); // the `this` (instance) slot
				$( <$arg as SqArg>::push_typemask(&mut typemask); )*
				let nparamscheck = 1i64 $(+ count_one!($arg))*;

				let mut func = self;
				let invoke = move |vm: &SQVM| -> squirrel_sys::SQInteger {
					let payload = resolve_receiver!(vm, name, T);
					#[allow(unused_mut)]
					let mut idx = 2i64;
					$(
						#[allow(non_snake_case)]
						let $arg = match <$arg as SqArg>::from_sq(vm, idx) {
							Ok(value) => value,
							Err(error) => return vm.throw_error(&error),
						};
						idx += 1;
					)*
					let _ = idx;
					// SAFETY: `payload` points into the receiver's heap payload (kept
					// alive on the stack for this call); args were already read, so this
					// `&mut T` does not alias a live borrow of the VM stack.
					let receiver: &mut T = unsafe { &mut (*payload).value };
					match func(receiver, $($arg),*).push_return(vm) {
						Ok(count) => count,
						Err(error) => vm.throw_error(&error),
					}
				};

				Box::new(MethodFn { invoke: Box::new(invoke), nparamscheck, typemask })
			}
		}

		// -------- vm-receiving: FnMut(&mut T, &SQVM, args..) -> R --------
		// The `&SQVM` is the VM context, not a Squirrel argument, so the schema is
		// identical to the plain impl and args still start at index 2.
		impl<T, Func, Ret, $($arg),*> IntoSqMethod<T, (VmRecv, $($arg,)*)> for Func
		where
			T: 'static,
			Func: for<'vm> FnMut(&mut T, &'vm SQVM, $($arg),*) -> Ret
				+ for<'a> FnMut(&mut T, &'a SQVM, $(<$arg as SqArg>::Item<'a>),*) -> Ret
				+ 'static,
			Ret: IntoSqReturn,
			$($arg: SqArg + 'static,)*
		{
			fn into_method_fn(self, name: String) -> Box<dyn SqFunction> {
				#[allow(unused_mut)]
				let mut typemask = String::from(".");
				$( <$arg as SqArg>::push_typemask(&mut typemask); )*
				let nparamscheck = 1i64 $(+ count_one!($arg))*;

				let mut func = self;
				let invoke = move |vm: &SQVM| -> squirrel_sys::SQInteger {
					let payload = resolve_receiver!(vm, name, T);
					#[allow(unused_mut)]
					let mut idx = 2i64;
					$(
						#[allow(non_snake_case)]
						let $arg = match <$arg as SqArg>::from_sq(vm, idx) {
							Ok(value) => value,
							Err(error) => return vm.throw_error(&error),
						};
						idx += 1;
					)*
					let _ = idx;
					// SAFETY: see the plain impl. The `&SQVM` and the payload heap are
					// disjoint, so the `&mut T` and `vm` borrows do not conflict.
					let receiver: &mut T = unsafe { &mut (*payload).value };
					match func(receiver, vm, $($arg),*).push_return(vm) {
						Ok(count) => count,
						Err(error) => vm.throw_error(&error),
					}
				};

				Box::new(MethodFn { invoke: Box::new(invoke), nparamscheck, typemask })
			}
		}
	};
}

impl_into_sq_method!();
impl_into_sq_method!(A0);
impl_into_sq_method!(A0, A1);
impl_into_sq_method!(A0, A1, A2);
impl_into_sq_method!(A0, A1, A2, A3);
impl_into_sq_method!(A0, A1, A2, A3, A4);
impl_into_sq_method!(A0, A1, A2, A3, A4, A5);
impl_into_sq_method!(A0, A1, A2, A3, A4, A5, A6);
impl_into_sq_method!(A0, A1, A2, A3, A4, A5, A6, A7);

// -------- variadic: FnMut(&mut T, Variadic<V>) -> R --------
// A method accepting any number of trailing args (e.g. no-op stub methods like
// `tile.spawnDetail(...)` / `tile.clear()` whose arity varies). Mirrors the
// `Variadic` tail of `IntoSqFunction`.
impl<T, Func, Ret, Var> IntoSqMethod<T, (VariadicRecv, Var)> for Func
where
	T: 'static,
	Func: FnMut(&mut T, Variadic<Var>) -> Ret
		+ for<'a> FnMut(&mut T, Variadic<<Var as SqArg>::Item<'a>>) -> Ret
		+ 'static,
	Ret: IntoSqReturn,
	Var: SqArg + 'static,
{
	fn into_method_fn(self, name: String) -> Box<dyn SqFunction> {
		let mut func = self;
		let invoke = move |vm: &SQVM| -> squirrel_sys::SQInteger {
			let payload = resolve_receiver!(vm, name, T);
			// Trailing args occupy stack idx 2 ..< top; the closure free variable
			// (userdata) sits at `top`, so it is excluded.
			let top = vm.stack_top();
			let mut rest = Vec::new();
			let mut idx = 2i64;
			while idx < top {
				match <Var as SqArg>::from_sq(vm, idx) {
					Ok(value) => rest.push(value),
					Err(error) => return vm.throw_error(&error),
				}
				idx += 1;
			}
			// SAFETY: see the fixed-arity impls.
			let receiver: &mut T = unsafe { &mut (*payload).value };
			match func(receiver, Variadic(rest)).push_return(vm) {
				Ok(count) => count,
				Err(error) => vm.throw_error(&error),
			}
		};
		Box::new(MethodFn {
			invoke: Box::new(invoke),
			nparamscheck: -1,
			typemask: String::from("."),
		})
	}
}

// -------- variadic with the VM: FnMut(&mut T, &SQVM, Variadic<V>) -> R --------
// For a variable-arity method that also builds a return object (e.g. a tactical
// tile's `spawnDetail(brush [, flags [, …]])`, which returns a decal handle).
impl<T, Func, Ret, Var> IntoSqMethod<T, (VmVariadicRecv, Var)> for Func
where
	T: 'static,
	Func: FnMut(&mut T, &SQVM, Variadic<Var>) -> Ret
		+ for<'a> FnMut(&mut T, &'a SQVM, Variadic<<Var as SqArg>::Item<'a>>) -> Ret
		+ 'static,
	Ret: IntoSqReturn,
	Var: SqArg + 'static,
{
	fn into_method_fn(self, name: String) -> Box<dyn SqFunction> {
		let mut func = self;
		let invoke = move |vm: &SQVM| -> squirrel_sys::SQInteger {
			let payload = resolve_receiver!(vm, name, T);
			let top = vm.stack_top();
			let mut rest = Vec::new();
			let mut idx = 2i64;
			while idx < top {
				match <Var as SqArg>::from_sq(vm, idx) {
					Ok(value) => rest.push(value),
					Err(error) => return vm.throw_error(&error),
				}
				idx += 1;
			}
			// SAFETY: see the fixed-arity impls.
			let receiver: &mut T = unsafe { &mut (*payload).value };
			match func(receiver, vm, Variadic(rest)).push_return(vm) {
				Ok(count) => count,
				Err(error) => vm.throw_error(&error),
			}
		};
		Box::new(MethodFn {
			invoke: Box::new(invoke),
			nparamscheck: -1,
			typemask: String::from("."),
		})
	}
}
