//! Registering Rust closures as Squirrel native functions.
//!
//! A plain closure becomes a native function via [`SQVM::register`]. The pieces:
//!   * [`arg`]      — [`SqArg`]: reading each parameter off the stack + its type mask
//!   * [`ret`]      — [`IntoSqReturn`]: pushing the return value (or throwing)
//!   * [`this`]     — [`This`]: the borrowed receiver, as a leading parameter
//!   * [`variadic`] — [`Variadic`]: a trailing "rest" parameter
//!   * [`forward`]  — [`StackArg`]/[`Forwarded`]: passing a call's arguments on unretained
//!   * [`into_fn`]  — the per-arity glue that derives the param-check schema and
//!                    wires a closure of each shape into a boxed [`SqFunction`]
//!
//! The trampoline below is the single `extern "C"` entry point Squirrel calls;
//! it recovers the boxed [`SqFunction`] from the closure's free variable and
//! dispatches to it.

mod arg;
mod forward;
mod into_fn;
mod number;
mod ret;
mod sqrat_args;
mod strong_ref;
mod this;
mod variadic;

#[cfg(test)]
mod tests;

pub use forward::{ForwardArgs, Forwarded, MemberKey, StackArg, StackArgs};
pub use number::{DynNumber, Number};
pub use sqrat_args::{Stringified, Truthy};
pub use strong_ref::{Retained, SqStrongRef};
pub use this::This;
pub use variadic::Variadic;

pub(crate) use arg::SqArg;
pub(crate) use into_fn::IntoSqFunction;
pub(crate) use ret::IntoSqReturn;
pub(crate) use strong_ref::CallArgs;

use std::ffi::CString;

use crate::{SQ_ERROR, SQ_OK, SQVM};

/// A native function callable from Squirrel.
///
/// This is the object-safe, VM-facing interface that lives behind the closure
/// userdata. Most callers should not implement it by hand — register a plain
/// Rust closure through [`SQVM::register`], which derives the param-check schema
/// and argument conversions automatically. Implement it directly only when you
/// need full manual control of the stack.
pub trait SqFunction {
	/// Read arguments from `vm`, run the function, and push its results.
	///
	/// Returns the number of values pushed onto the stack, or the sentinel from
	/// [`SQVM::throw_error`] to signal that a Squirrel exception was raised.
	fn invoke(&mut self, vm: &SQVM) -> squirrel_sys::SQInteger;

	/// Parameter-count policy handed to `sq_setparamscheck`.
	///
	/// `> 0` requires exactly that many parameters, `< 0` requires at least
	/// `abs(n)` (i.e. a variadic tail), and `0` disables the count check. The
	/// hidden `this` parameter is included in the count; bound free variables
	/// are not.
	fn nparamscheck(&self) -> i64 {
		0
	}

	/// Type mask handed to `sq_setparamscheck`, or `None` to skip type checking.
	///
	/// One character per parameter, where the first character matches `this`.
	/// See `sq_setparamscheck` for the type letters (`i`, `f`, `n`, `s`, `b`,
	/// `.`, …) and `|` for unions.
	fn typemask(&self) -> Option<&str> {
		None
	}

	/// What the profiler calls this function: by default its type's name.
	fn label(&self) -> &str {
		std::any::type_name::<Self>()
	}

	/// The function is registered by code at `at` (named in its [`label`](Self::label)).
	fn set_location(&mut self, _at: &'static std::panic::Location<'static>) {}
}

unsafe extern "C" fn trampoline(vm: squirrel_sys::HSQUIRRELVM) -> squirrel_sys::SQInteger {
	let vm = unsafe { SQVM::from_raw(vm) };

	// The boxed `SqFunction` is the single free variable bound by
	// `sq_newclosure`, so it sits at the very top of the stack (above `this`
	// and the call arguments).
	let top = unsafe { squirrel_sys::sq_gettop(vm.as_ptr()) };
	let mut ptr = std::ptr::null_mut();
	let res =
		unsafe { squirrel_sys::sq_getuserdata(vm.as_ptr(), top, &mut ptr, std::ptr::null_mut()) };
	if res != SQ_OK {
		return SQ_ERROR;
	}

	let func = unsafe { &mut *(ptr as *mut Box<dyn SqFunction>) };
	let Some(depth) = crate::profile::native_enter(func.label()) else { return func.invoke(&vm) };
	let result = func.invoke(&vm);
	crate::profile::native_exit(depth);
	result
}

impl SQVM {
	/// Build a native closure from `func` and leave it on top of the stack,
	/// applying its param-check schema. Shared by [`register`](Self::register)
	/// (which then slots it into the root table), [`make_closure`](Self::make_closure)
	/// (which hands it back as a value), and the native-class builder (which slots
	/// metamethods onto a class).
	pub(crate) fn push_closure(&mut self, func: Box<dyn SqFunction>) {
		// Capture the param-check schema before `func` is moved into userdata.
		let nparamscheck = func.nparamscheck();
		let typemask = func
			.typemask()
			.map(|mask| CString::new(mask).expect("typemask contained an interior NUL byte"));

		self.squirrel_alloc(func);
		unsafe {
			// 1 free variable: the boxed `SqFunction` allocated just above.
			squirrel_sys::sq_newclosure(self.as_ptr(), Some(trampoline), 1);

			// Apply the schema to the native closure now sitting on top of the
			// stack. Squirrel compiles and copies the typemask, so the temporary
			// `CString` does not need to outlive this call.
			if nparamscheck != 0 || typemask.is_some() {
				let mask_ptr = typemask
					.as_ref()
					.map_or(std::ptr::null(), |mask| mask.as_ptr());
				squirrel_sys::sq_setparamscheck(self.as_ptr(), nparamscheck, mask_ptr);
			}
		}
	}

	fn push_native_function<'a>(&'a mut self, func: Box<dyn SqFunction>, name: &'a str) {
		unsafe { squirrel_sys::sq_pushroottable(self.as_ptr()) };
		self.push_str(name);
		self.push_closure(func); // [root, name, closure]
		unsafe {
			squirrel_sys::sq_newslot(self.as_ptr(), -3, 0);
			squirrel_sys::sq_pop(self.as_ptr(), 1);
		}
	}

	/// Build a Rust closure into a native Squirrel function and return it as an
	/// owned [`SqStrongRef`], rather than binding it to a global name.
	///
	/// Use this to place a native function somewhere other than the root table —
	/// e.g. as a slot on a sub-table (`IO.scriptHashByFilename`, `Math.rand`, …):
	///
	/// ```ignore
	/// let f = vm.make_closure(|a: i64, b: i64| a + b);
	/// io_table.set("add", f);                 // SqStrongRef is an SqValue
	/// ```
	///
	/// The schema (arity + type mask) is derived from the signature exactly as for
	/// [`register`](Self::register).
	#[track_caller]
	pub fn make_closure<Marker, Func>(&mut self, func: Func) -> SqStrongRef
	where
		Func: IntoSqFunction<Marker>,
	{
		let mut func = func.into_sq_function();
		func.set_location(std::panic::Location::caller());
		self.push_closure(func); // [closure]
		let closure = SqStrongRef::from_stack(self, -1);
		unsafe { squirrel_sys::sq_pop(self.as_ptr(), 1) };
		closure
	}

	/// Register a Rust closure as a global Squirrel function named `name`.
	///
	/// The parameter count and type mask are derived from the closure's
	/// signature, so Squirrel rejects calls with the wrong arity or argument
	/// types before the closure body runs. End the parameter list with a
	/// [`Variadic`] to accept a variable number of trailing arguments, return a
	/// [`Result`] to raise a Squirrel exception, and take/return [`Option`] for
	/// nullable values.
	///
	/// ```ignore
	/// vm.register("add", |a: i64, b: i64| a + b);
	/// vm.register("greet", |name: String| format!("hi {name}"));
	/// vm.register("sum", |Variadic(xs): Variadic<i64>| xs.into_iter().sum::<i64>());
	/// ```
	#[track_caller]
	pub fn register<Marker, Func>(&mut self, name: &str, func: Func)
	where
		Func: IntoSqFunction<Marker>,
	{
		let mut func = func.into_sq_function();
		func.set_location(std::panic::Location::caller());
		self.push_native_function(func, name);
	}
}
