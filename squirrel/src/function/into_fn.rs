//! The per-arity glue that turns a Rust closure into a boxed [`SqFunction`].
//!
//! [`IntoSqFunction`] is implemented for every closure shape via a macro. Each
//! parameter's [`SqArg`] knows its type-mask char and how to read itself; the
//! return's [`IntoSqReturn`] pushes the result. The `Marker` type parameter
//! encodes the argument shape so the blanket impls never overlap; callers never
//! name it.
//!
//! Borrowed args (`&str` and the like) rely on a double `FnMut` bound; see the
//! fixed-arity arm of the macro below.

use super::{IntoSqReturn, SqArg, SqFunction, StackArgs, This, Variadic};
use crate::SQVM;

// Distinct markers so the blanket impls below target different
// `IntoSqFunction<Marker>` instantiations and never overlap. The marker tuple's
// head selects the shape: `FixedMarker`/`VariadicMarker` for plain closures, a
// leading `ThisMarker` for closures whose first parameter is `This`, and a
// leading `VmMarker` for closures whose first parameter is `&SQVM` (the VM
// context, for native functions that build tables / run scripts).
#[doc(hidden)]
pub struct FixedMarker;
#[doc(hidden)]
pub struct VariadicMarker;
#[doc(hidden)]
pub struct StackArgsMarker;
#[doc(hidden)]
pub struct ThisMarker;
#[doc(hidden)]
pub struct VmMarker;

/// Converts a Rust callable into a boxed [`SqFunction`], deriving its
/// param-check schema from the parameter and return types. `Marker` encodes the
/// argument shape so the per-arity impls don't overlap; callers never name it.
pub trait IntoSqFunction<Marker> {
	fn into_sq_function(self) -> Box<dyn SqFunction>;
}

/// The concrete [`SqFunction`] produced for a registered closure: the captured
/// closure plus its precomputed param-check schema.
struct ClosureFn {
	invoke: Box<dyn FnMut(&SQVM) -> squirrel_sys::SQInteger>,
	nparamscheck: i64,
	typemask: String,
	/// What the profiler calls it: the closure's type name (which names the function
	/// that made it) and where it was registered.
	label: String,
}

impl SqFunction for ClosureFn {
	fn invoke(&mut self, vm: &SQVM) -> squirrel_sys::SQInteger {
		(self.invoke)(vm)
	}
	fn nparamscheck(&self) -> i64 {
		self.nparamscheck
	}
	fn typemask(&self) -> Option<&str> {
		Some(self.typemask.as_str())
	}
	fn label(&self) -> &str {
		&self.label
	}
	fn set_location(&mut self, at: &'static std::panic::Location<'static>) {
		self.label = format!("{} at {}:{}", self.label, at.file(), at.line());
	}
}

/// Expands to `1` for each argument, used to count fixed parameters.
macro_rules! count_one {
	($t:ident) => {
		1i64
	};
}

macro_rules! impl_into_sq_function {
	($($arg:ident),*) => {
		// -------- fixed arity: FnMut(A, B, ...) -> R --------
		impl<Func, Ret, $($arg),*> IntoSqFunction<(FixedMarker, $($arg,)*)> for Func
		where
			// The first bound anchors inference of each `$arg` to the closure's
			// parameter types; the second is the one actually called, with each
			// argument re-borrowed for the call's lifetime via `SqArg::Item`.
			Func: FnMut($($arg),*) -> Ret
				+ for<'a> FnMut($(<$arg as SqArg>::Item<'a>),*) -> Ret
				+ 'static,
			Ret: IntoSqReturn,
			$($arg: SqArg + 'static,)*
		{
			fn into_sq_function(self) -> Box<dyn SqFunction> {
				#[allow(unused_mut)]
				let mut typemask = String::from(".");
				$( <$arg as SqArg>::push_typemask(&mut typemask); )*
				// `this` plus the fixed parameters.
				let nparamscheck = 1i64 $(+ count_one!($arg))*;

				let mut func = self;
				let invoke = move |vm: &SQVM| -> squirrel_sys::SQInteger {
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
					match func($($arg),*).push_return(vm) {
						Ok(count) => count,
						Err(error) => vm.throw_error(&error),
					}
				};

				Box::new(ClosureFn { invoke: Box::new(invoke), nparamscheck, typemask, label: std::any::type_name::<Func>().to_string() })
			}
		}

		// -------- variadic tail: FnMut(A, B, ..., Variadic<V>) -> R --------
		impl<Func, Ret, Var, $($arg),*> IntoSqFunction<(VariadicMarker, Var, $($arg,)*)> for Func
		where
			Func: FnMut($($arg,)* Variadic<Var>) -> Ret
				+ for<'a> FnMut($(<$arg as SqArg>::Item<'a>,)* Variadic<<Var as SqArg>::Item<'a>>) -> Ret
				+ 'static,
			Ret: IntoSqReturn,
			Var: SqArg + 'static,
			$($arg: SqArg + 'static,)*
		{
			fn into_sq_function(self) -> Box<dyn SqFunction> {
				#[allow(unused_mut)]
				let mut typemask = String::from(".");
				$( <$arg as SqArg>::push_typemask(&mut typemask); )*
				// Negative => "at least" `this` plus the fixed parameters.
				let nparamscheck = -(1i64 $(+ count_one!($arg))*);

				let mut func = self;
				let invoke = move |vm: &SQVM| -> squirrel_sys::SQInteger {
					let mut idx = 2i64;
					$(
						#[allow(non_snake_case)]
						let $arg = match <$arg as SqArg>::from_sq(vm, idx) {
							Ok(value) => value,
							Err(error) => return vm.throw_error(&error),
						};
						idx += 1;
					)*
					// Trailing arguments occupy `idx ..= top - 1`; the free
					// variable (closure userdata) sits at `top`.
					let top = vm.stack_top();
					let mut rest = Vec::new();
					while idx < top {
						match <Var as SqArg>::from_sq(vm, idx) {
							Ok(value) => rest.push(value),
							Err(error) => return vm.throw_error(&error),
						}
						idx += 1;
					}
					match func($($arg,)* Variadic(rest)).push_return(vm) {
						Ok(count) => count,
						Err(error) => vm.throw_error(&error),
					}
				};

				Box::new(ClosureFn { invoke: Box::new(invoke), nparamscheck, typemask, label: std::any::type_name::<Func>().to_string() })
			}
		}

		// -------- this + fixed: FnMut(This, A, B, ...) -> R --------
		// Same schema as the plain fixed impl: `this` is already counted and its
		// type-mask slot stays `.`, so `nparamscheck`/`typemask` are byte-identical.
		// `This::extract` reads index 1; the argument loop still starts at index 2.
		impl<Func, Ret, $($arg),*> IntoSqFunction<(ThisMarker, FixedMarker, $($arg,)*)> for Func
		where
			Func: for<'vm> FnMut(This<'vm>, $($arg),*) -> Ret
				+ for<'a> FnMut(This<'a>, $(<$arg as SqArg>::Item<'a>),*) -> Ret
				+ 'static,
			Ret: IntoSqReturn,
			$($arg: SqArg + 'static,)*
		{
			fn into_sq_function(self) -> Box<dyn SqFunction> {
				#[allow(unused_mut)]
				let mut typemask = String::from(".");
				$( <$arg as SqArg>::push_typemask(&mut typemask); )*
				let nparamscheck = 1i64 $(+ count_one!($arg))*;

				let mut func = self;
				let invoke = move |vm: &SQVM| -> squirrel_sys::SQInteger {
					let this = This::extract(vm);
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
					match func(this, $($arg),*).push_return(vm) {
						Ok(count) => count,
						Err(error) => vm.throw_error(&error),
					}
				};

				Box::new(ClosureFn { invoke: Box::new(invoke), nparamscheck, typemask, label: std::any::type_name::<Func>().to_string() })
			}
		}

		// -------- this + variadic: FnMut(This, A, B, ..., Variadic<V>) -> R --------
		impl<Func, Ret, Var, $($arg),*> IntoSqFunction<(ThisMarker, VariadicMarker, Var, $($arg,)*)>
			for Func
		where
			Func: for<'vm> FnMut(This<'vm>, $($arg,)* Variadic<Var>) -> Ret
				+ for<'a> FnMut(This<'a>, $(<$arg as SqArg>::Item<'a>,)* Variadic<<Var as SqArg>::Item<'a>>) -> Ret
				+ 'static,
			Ret: IntoSqReturn,
			Var: SqArg + 'static,
			$($arg: SqArg + 'static,)*
		{
			fn into_sq_function(self) -> Box<dyn SqFunction> {
				#[allow(unused_mut)]
				let mut typemask = String::from(".");
				$( <$arg as SqArg>::push_typemask(&mut typemask); )*
				let nparamscheck = -(1i64 $(+ count_one!($arg))*);

				let mut func = self;
				let invoke = move |vm: &SQVM| -> squirrel_sys::SQInteger {
					let this = This::extract(vm);
					let mut idx = 2i64;
					$(
						#[allow(non_snake_case)]
						let $arg = match <$arg as SqArg>::from_sq(vm, idx) {
							Ok(value) => value,
							Err(error) => return vm.throw_error(&error),
						};
						idx += 1;
					)*
					let top = vm.stack_top();
					let mut rest = Vec::new();
					while idx < top {
						match <Var as SqArg>::from_sq(vm, idx) {
							Ok(value) => rest.push(value),
							Err(error) => return vm.throw_error(&error),
						}
						idx += 1;
					}
					match func(this, $($arg,)* Variadic(rest)).push_return(vm) {
						Ok(count) => count,
						Err(error) => vm.throw_error(&error),
					}
				};

				Box::new(ClosureFn { invoke: Box::new(invoke), nparamscheck, typemask, label: std::any::type_name::<Func>().to_string() })
			}
		}

		// -------- &SQVM + fixed: FnMut(&SQVM, A, B, ...) -> R --------
		// The leading `&SQVM` is the VM context, not a Squirrel argument, so the
		// schema matches the plain fixed impl and args still start at index 2.
		// It lets native functions build tables / run scripts via the public
		// high-level API without ever touching the stack.
		impl<Func, Ret, $($arg),*> IntoSqFunction<(VmMarker, FixedMarker, $($arg,)*)> for Func
		where
			Func: for<'vm> FnMut(&'vm SQVM, $($arg),*) -> Ret
				+ for<'a> FnMut(&'a SQVM, $(<$arg as SqArg>::Item<'a>),*) -> Ret
				+ 'static,
			Ret: IntoSqReturn,
			$($arg: SqArg + 'static,)*
		{
			fn into_sq_function(self) -> Box<dyn SqFunction> {
				#[allow(unused_mut)]
				let mut typemask = String::from(".");
				$( <$arg as SqArg>::push_typemask(&mut typemask); )*
				let nparamscheck = 1i64 $(+ count_one!($arg))*;

				let mut func = self;
				let invoke = move |vm: &SQVM| -> squirrel_sys::SQInteger {
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
					match func(vm, $($arg),*).push_return(vm) {
						Ok(count) => count,
						Err(error) => vm.throw_error(&error),
					}
				};

				Box::new(ClosureFn { invoke: Box::new(invoke), nparamscheck, typemask, label: std::any::type_name::<Func>().to_string() })
			}
		}

		// -------- &SQVM + this + fixed: FnMut(&SQVM, This, A, B, ...) -> R --------
		// For a native that keeps its receiver (`This::to_strong` needs the VM), e.g. to
		// call a script callback later with the receiver as `this`, as Sqrat does.
		impl<Func, Ret, $($arg),*> IntoSqFunction<(VmMarker, ThisMarker, FixedMarker, $($arg,)*)> for Func
		where
			Func: for<'vm> FnMut(&'vm SQVM, This<'vm>, $($arg),*) -> Ret
				+ for<'a> FnMut(&'a SQVM, This<'a>, $(<$arg as SqArg>::Item<'a>),*) -> Ret
				+ 'static,
			Ret: IntoSqReturn,
			$($arg: SqArg + 'static,)*
		{
			fn into_sq_function(self) -> Box<dyn SqFunction> {
				#[allow(unused_mut)]
				let mut typemask = String::from(".");
				$( <$arg as SqArg>::push_typemask(&mut typemask); )*
				let nparamscheck = 1i64 $(+ count_one!($arg))*;

				let mut func = self;
				let invoke = move |vm: &SQVM| -> squirrel_sys::SQInteger {
					let this = This::extract(vm);
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
					match func(vm, this, $($arg),*).push_return(vm) {
						Ok(count) => count,
						Err(error) => vm.throw_error(&error),
					}
				};

				Box::new(ClosureFn { invoke: Box::new(invoke), nparamscheck, typemask, label: std::any::type_name::<Func>().to_string() })
			}
		}

		// -------- &SQVM + variadic: FnMut(&SQVM, A, ..., Variadic<V>) -> R --------
		impl<Func, Ret, Var, $($arg),*> IntoSqFunction<(VmMarker, VariadicMarker, Var, $($arg,)*)>
			for Func
		where
			Func: for<'vm> FnMut(&'vm SQVM, $($arg,)* Variadic<Var>) -> Ret
				+ for<'a> FnMut(&'a SQVM, $(<$arg as SqArg>::Item<'a>,)* Variadic<<Var as SqArg>::Item<'a>>) -> Ret
				+ 'static,
			Ret: IntoSqReturn,
			Var: SqArg + 'static,
			$($arg: SqArg + 'static,)*
		{
			fn into_sq_function(self) -> Box<dyn SqFunction> {
				#[allow(unused_mut)]
				let mut typemask = String::from(".");
				$( <$arg as SqArg>::push_typemask(&mut typemask); )*
				let nparamscheck = -(1i64 $(+ count_one!($arg))*);

				let mut func = self;
				let invoke = move |vm: &SQVM| -> squirrel_sys::SQInteger {
					let mut idx = 2i64;
					$(
						#[allow(non_snake_case)]
						let $arg = match <$arg as SqArg>::from_sq(vm, idx) {
							Ok(value) => value,
							Err(error) => return vm.throw_error(&error),
						};
						idx += 1;
					)*
					let top = vm.stack_top();
					let mut rest = Vec::new();
					while idx < top {
						match <Var as SqArg>::from_sq(vm, idx) {
							Ok(value) => rest.push(value),
							Err(error) => return vm.throw_error(&error),
						}
						idx += 1;
					}
					match func(vm, $($arg,)* Variadic(rest)).push_return(vm) {
						Ok(count) => count,
						Err(error) => vm.throw_error(&error),
					}
				};

				Box::new(ClosureFn { invoke: Box::new(invoke), nparamscheck, typemask, label: std::any::type_name::<Func>().to_string() })
			}
		}
	};
}

impl_into_sq_function!();
impl_into_sq_function!(A0);
impl_into_sq_function!(A0, A1);
impl_into_sq_function!(A0, A1, A2);
impl_into_sq_function!(A0, A1, A2, A3);
impl_into_sq_function!(A0, A1, A2, A3, A4);
impl_into_sq_function!(A0, A1, A2, A3, A4, A5);
impl_into_sq_function!(A0, A1, A2, A3, A4, A5, A6);
impl_into_sq_function!(A0, A1, A2, A3, A4, A5, A6, A7);

// `&SQVM` + `This` + `Variadic`, with no fixed args: `FnMut(&SQVM, This, Variadic<V>)`.
// This is the exact shape of a Sqrat-style overload dispatcher — it needs the VM (to
// re-resolve and call the chosen `__sqrat_ol_` leaf), the receiver (`this`, to resolve
// the leaf against, honouring any mod replacement), and the variadic tail (to count the
// arity and forward the args). Only the zero-fixed-arg form is needed, so it is a single
// standalone impl rather than another arm of the per-arity macro. Its marker tuple
// `(VmMarker, ThisMarker, VariadicMarker, Var)` is distinct from every macro-generated
// tuple, so there is no overlap.
impl<Func, Ret, Var> IntoSqFunction<(VmMarker, ThisMarker, VariadicMarker, Var)> for Func
where
	Func: for<'vm> FnMut(&'vm SQVM, This<'vm>, Variadic<Var>) -> Ret
		+ for<'a> FnMut(&'a SQVM, This<'a>, Variadic<<Var as SqArg>::Item<'a>>) -> Ret
		+ 'static,
	Ret: IntoSqReturn,
	Var: SqArg + 'static,
{
	fn into_sq_function(self) -> Box<dyn SqFunction> {
		let typemask = String::from("."); // only `this`; the tail is unconstrained
		let nparamscheck = -1i64; // "at least `this`" — any number of trailing args
		let mut func = self;
		let invoke = move |vm: &SQVM| -> squirrel_sys::SQInteger {
			let this = This::extract(vm);
			let top = vm.stack_top();
			let mut rest = Vec::new();
			let mut idx = 2i64; // skip `this` at index 1; stop before the bound free var
			while idx < top {
				match <Var as SqArg>::from_sq(vm, idx) {
					Ok(value) => rest.push(value),
					Err(error) => return vm.throw_error(&error),
				}
				idx += 1;
			}
			match func(vm, this, Variadic(rest)).push_return(vm) {
				Ok(count) => count,
				Err(error) => vm.throw_error(&error),
			}
		};
		Box::new(ClosureFn {
			invoke: Box::new(invoke),
			nparamscheck,
			typemask,
			label: std::any::type_name::<Func>().to_string(),
		})
	}
}

// `&SQVM` + `This` + `StackArgs`: a forwarding dispatcher that takes any number of
// arguments and passes them on in place (see `This::forward_member`), with no per-call
// collection.
impl<Func, Ret> IntoSqFunction<(VmMarker, ThisMarker, StackArgsMarker)> for Func
where
	Func: for<'vm> FnMut(&'vm SQVM, This<'vm>, StackArgs<'vm>) -> Ret + 'static,
	Ret: IntoSqReturn,
{
	fn into_sq_function(self) -> Box<dyn SqFunction> {
		let typemask = String::from("."); // only `this`; the tail is unconstrained
		let nparamscheck = -1i64; // "at least `this`" — any number of trailing args
		let mut func = self;
		let invoke = move |vm: &SQVM| -> squirrel_sys::SQInteger {
			let this = This::extract(vm);
			// Arguments occupy `2 ..= top - 1`; the free variable (closure userdata)
			// sits at `top`.
			let top = vm.stack_top();
			let args = StackArgs::new(2, (top - 2).max(0) as usize);
			match func(vm, this, args).push_return(vm) {
				Ok(count) => count,
				Err(error) => vm.throw_error(&error),
			}
		};
		Box::new(ClosureFn {
			invoke: Box::new(invoke),
			nparamscheck,
			typemask,
			label: std::any::type_name::<Func>().to_string(),
		})
	}
}
