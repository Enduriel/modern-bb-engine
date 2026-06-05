//! Recursive (deep) copying of Squirrel values at the stack level.
//!
//! [`push_deep_clone`] pushes a deep copy of a value: tables and arrays are
//! rebuilt slot-by-slot (recursively), so the copy shares no mutable container
//! with the original. Every other type (scalars, strings, closures, instances,
//! …) is pushed by reference — there is nothing mutable to alias. This is the
//! primitive behind [`SqTable::deep_clone`](super::SqTable::deep_clone) and
//! [`SqArray::deep_clone`](super::SqArray::deep_clone), which the BB module
//! system uses to give every instance its own member (`m`) table.
//!
//! Assumes the value is acyclic (as configuration / `m`-default data is): a cycle
//! would recurse without end.

use crate::{SQ_OK, SQVM, SqObjectType};

/// Push a deep clone of the value at `idx` onto the top of the stack.
///
/// Net stack effect: `+1` (the clone). Tables/arrays are cloned recursively;
/// all other values are pushed by reference (an addref for ref types).
pub(crate) unsafe fn push_deep_clone(vm: &SQVM, idx: i64) {
	let ptr = vm.as_ptr();
	// Resolve to an absolute index up front: the table/array branches push onto
	// the stack while iterating, which would shift a relative `idx`.
	let top = unsafe { squirrel_sys::sq_gettop(ptr) };
	let src = if idx < 0 { top + idx + 1 } else { idx };

	match vm.get_type(src) {
		SqObjectType::Table => unsafe {
			squirrel_sys::sq_newtable(ptr); // [.., new]
			let new_idx = squirrel_sys::sq_gettop(ptr);
			squirrel_sys::sq_pushnull(ptr); // [.., new, iter]
			while squirrel_sys::sq_next(ptr, src) == SQ_OK {
				// [.., new, iter, key, value]
				push_deep_clone(vm, -1); // [.., new, iter, key, value, clone]
				squirrel_sys::sq_remove(ptr, -2); // drop original value -> [.., key, clone]
				// newslot pops key + clone, writes into the table at new_idx.
				squirrel_sys::sq_newslot(ptr, new_idx, 0); // [.., new, iter]
			}
			squirrel_sys::sq_pop(ptr, 1); // pop iterator -> [.., new]
		},
		SqObjectType::Array => unsafe {
			squirrel_sys::sq_newarray(ptr, 0); // [.., new]
			let new_idx = squirrel_sys::sq_gettop(ptr);
			squirrel_sys::sq_pushnull(ptr); // [.., new, iter]
			while squirrel_sys::sq_next(ptr, src) == SQ_OK {
				// [.., new, iter, index, value]
				push_deep_clone(vm, -1); // [.., new, iter, index, value, clone]
				squirrel_sys::sq_remove(ptr, -2); // drop original value -> [.., index, clone]
				squirrel_sys::sq_remove(ptr, -2); // drop the index key -> [.., clone]
				squirrel_sys::sq_arrayappend(ptr, new_idx); // pops clone -> [.., new, iter]
			}
			squirrel_sys::sq_pop(ptr, 1); // pop iterator -> [.., new]
		},
		// Scalars, strings and reference types: copy by reference. `sq_push`
		// duplicates the stack slot (an addref for ref types).
		_ => unsafe { squirrel_sys::sq_push(ptr, src) },
	}
}
