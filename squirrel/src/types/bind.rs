//! Closure environment binding (Squirrel's `bindenv`): a bound closure runs with `this`
//! fixed to its environment, whatever object it is looked up on or `.call`ed with.
//!
//! The BB class system needs this: the original engine binds every method of every
//! class level to the instance's leaf, so a super-call (`this.actor.onInit()`) still
//! runs with `this` = the full instance.

use super::SqTable;
use crate::{SQ_OK, SQVM, SqObjectType, SqStrongRef};

impl SqStrongRef {
	/// A copy of this closure (script or native) bound to `env`, or `None` if the
	/// referenced object is not a closure. The binding holds `env` weakly, as
	/// Squirrel's `bindenv` does, so it creates no reference cycle.
	pub fn bindenv(&self, vm: &SQVM, env: &SqTable) -> Option<SqStrongRef> {
		let ptr = vm.as_ptr();
		self.push(vm); // [c]
		if !matches!(vm.get_type(-1), SqObjectType::Closure | SqObjectType::NativeClosure) {
			unsafe { squirrel_sys::sq_pop(ptr, 1) };
			return None;
		}
		unsafe {
			squirrel_sys::sq_pushobject(ptr, env.obj); // [c, env]
			if squirrel_sys::sq_bindenv(ptr, -2) == SQ_OK {
				// sq_bindenv popped env and pushed the bound copy -> [c, bound]
				let bound = SqStrongRef::from_stack(vm, -1);
				squirrel_sys::sq_pop(ptr, 2);
				Some(bound)
			} else {
				// Only fails for a non-closure target or invalid env (both excluded).
				squirrel_sys::sq_pop(ptr, 2);
				None
			}
		}
	}
}

impl SqStrongRef {
	/// Battle Brothers compatibility: make this script closure (and every function
	/// nested in it) compute float arithmetic in single precision and round its float
	/// literals to single precision, as the original 32-bit-float engine did. Values
	/// remain stored as 64-bit floats. Returns `false` if this is not a script closure.
	pub fn set_f32_math(&self, vm: &SQVM, on: bool) -> bool {
		let ptr = vm.as_ptr();
		self.push(vm);
		let ok = unsafe { squirrel_sys::sq_setf32math(ptr, -1, u64::from(on) as _) } == SQ_OK;
		unsafe {
			if !ok {
				squirrel_sys::sq_reseterror(ptr);
			}
			squirrel_sys::sq_pop(ptr, 1);
		}
		ok
	}
}

impl SqTable<'_> {
	/// Replace every closure slot in this table (own slots with string keys only, not
	/// the delegate's) with a copy bound to `env`. Other slots are left untouched.
	///
	/// Runs as one native walk: each closure is bound and stored back under the key
	/// object the iteration already holds, so no key is re-interned and no reference
	/// is retained. Overwriting an existing key's value never rehashes the table, so
	/// it is safe while iterating.
	pub fn bind_closures_to(&mut self, env: &SqTable) {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [t]
			let table = squirrel_sys::sq_gettop(ptr);
			squirrel_sys::sq_pushnull(ptr); // [t, iter]
			while squirrel_sys::sq_next(ptr, table) == SQ_OK {
				// [t, iter, key, value]
				let closure = matches!(
					self.vm.get_type(-1),
					SqObjectType::Closure | SqObjectType::NativeClosure
				);
				if closure && self.vm.get_type(-2) == SqObjectType::String {
					squirrel_sys::sq_pushobject(ptr, env.obj); // [.., key, value, env]
					if squirrel_sys::sq_bindenv(ptr, -2) == SQ_OK {
						// popped env, pushed the bound copy -> [.., key, value, bound]
						squirrel_sys::sq_push(ptr, -3); // [.., key, value, bound, key]
						squirrel_sys::sq_push(ptr, -2); // [.., key, value, bound, key, bound]
						squirrel_sys::sq_rawset(ptr, table); // pops key + bound
						squirrel_sys::sq_pop(ptr, 1); // [.., key, value]
					} else {
						// Only fails for a non-closure target or invalid env (both
						// excluded); env is still on the stack.
						squirrel_sys::sq_reseterror(ptr);
						squirrel_sys::sq_pop(ptr, 1); // [.., key, value]
					}
				}
				squirrel_sys::sq_pop(ptr, 2); // [t, iter]
			}
			squirrel_sys::sq_pop(ptr, 2); // []
		}
	}
}

#[cfg(test)]
mod tests {
	use crate::{SQVM, SqTable};

	#[test]
	fn f32_math_reproduces_the_original_32_bit_engine() {
		let vm = SQVM::new(1024);
		// Results read from the original game.
		let check = r#"
			local H = 170;
			assert((H * 0.7).tointeger() == 119);           // f64 gives 118
			assert(16777217.0 - 16777216.0 == 0.0);         // beyond f32 precision
			assert(0.1 + 0.2 == 0.3);
			local y = 25.5; y++; assert(y == 26.5);         // increments stay f32
			local x = 3; x += 0.1; assert(x == 3.1);        // compound assignment literal rounding
			local f = function() { return 170 * 0.7; };     // nested functions inherit the mode
			assert(f().tointeger() == 119);
		"#;
		let flagged = vm.compile(check, "f32").unwrap();
		assert!(flagged.set_f32_math(&vm, true));
		flagged.call(&vm, ()).expect("f32 semantics");
		// Unflagged code keeps full 64-bit floats.
		vm.run_script("assert((170 * 0.7).tointeger() == 118); assert(0.1 + 0.2 != 0.3);", "f64")
			.expect("f64 semantics");
	}

	#[test]
	fn bound_methods_keep_this_across_lookup_and_call() {
		let vm = SQVM::new(1024);
		vm.run_script(
			r#"
			::leaf <- { name = "leaf" };
			::level <- { name = "level", function who() { return this.name; } };
			"#,
			"setup",
		)
		.unwrap();
		let root = SqTable::root(&vm);
		let leaf = root.get::<SqTable>("leaf").unwrap();
		let mut level = root.get::<SqTable>("level").unwrap();
		level.bind_closures_to(&leaf);
		vm.run_script(
			r#"
			assert(::level.who() == "leaf");               // looked up on another table
			assert(::level.who.call({ name = "x" }) == "leaf"); // explicit this ignored
			assert(::level.name == "level");               // data slots untouched
			"#,
			"check",
		)
		.expect("bound closures run with this = leaf");
	}
}
