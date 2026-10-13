//! [`SqTable`]: a map-like handle over a live Squirrel table.

use super::deep::push_deep_clone;
use super::{SqRef, SqValue};
use crate::{CallArgs, SQ_OK, SQVM, SqObjectType, SqStrongRef};

/// A borrowed, map-like handle to a live Squirrel table (string-keyed).
///
/// The sibling of [`SqArray`](super::SqArray): same borrowed model (holds an
/// addref'd `HSQOBJECT`, releases on `Drop`, `'vm` ties it to the VM borrow),
/// operating on the *actual* Squirrel table so changes are visible to scripts and
/// vice-versa. Keys are `&str`; values convert via [`SqValue`], so `get::<i64>`
/// yields an owned value while `get::<SqTable>` / `get::<SqArray>` yield nested
/// handles. Reads follow the **delegate chain** (like `t[key]` in a script, and
/// like BB's `inherit` chain), so an inherited key resolves; each read is one
/// level deep (a nested table/array comes back as a handle — cycle-safe).
///
/// ```ignore
/// let mut t = SqTable::new(&vm);
/// t.set("hp", 50i64);
/// assert_eq!(t.get::<i64>("hp"), Some(50));
/// let globals = SqTable::root(&vm);          // the root table itself
/// globals.set("MyArray", SqArray::new(&vm));
/// ```
pub struct SqTable<'vm> {
	pub(super) vm: &'vm SQVM,
	pub(super) obj: squirrel_sys::HSQOBJECT,
}

impl<'vm> SqTable<'vm> {
	/// Create a new, empty Squirrel table.
	pub fn new(vm: &'vm SQVM) -> Self {
		let ptr = vm.as_ptr();
		let mut obj: squirrel_sys::HSQOBJECT = unsafe { std::mem::zeroed() };
		unsafe {
			squirrel_sys::sq_newtable(ptr); // [table]
			squirrel_sys::sq_getstackobj(ptr, -1, &mut obj);
			squirrel_sys::sq_addref(ptr, &mut obj);
			squirrel_sys::sq_pop(ptr, 1); // []
		}
		SqTable { vm, obj }
	}

	/// A handle to the VM's root table (where globals live).
	pub fn root(vm: &'vm SQVM) -> Self {
		let ptr = vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushroottable(ptr); // [root]
			let table = SqTable::from_stack(vm, -1);
			squirrel_sys::sq_pop(ptr, 1);
			// The root is always a table.
			table.expect("root table")
		}
	}

	/// Wrap the table value at stack `idx` as an owned handle (addref'd), or
	/// `None` if that slot is not a table. Does not change the stack.
	pub(crate) fn from_stack(vm: &'vm SQVM, idx: i64) -> Option<Self> {
		if !matches!(vm.get_type(idx), SqObjectType::Table) {
			return None;
		}
		let mut obj: squirrel_sys::HSQOBJECT = unsafe { std::mem::zeroed() };
		unsafe {
			squirrel_sys::sq_getstackobj(vm.as_ptr(), idx, &mut obj);
			squirrel_sys::sq_addref(vm.as_ptr(), &mut obj);
		}
		Some(SqTable { vm, obj })
	}

	/// Number of slots.
	pub fn len(&self) -> usize {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj);
			let n = squirrel_sys::sq_getsize(ptr, -1);
			squirrel_sys::sq_pop(ptr, 1);
			if n < 0 { 0 } else { n as usize }
		}
	}

	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}

	/// The value at `key` read as `V`, or `None` if absent or not a `V`. Follows
	/// the delegate chain (like `t[key]` in a script), so an inherited key
	/// resolves; the implicit root-table fallback is *not* applied.
	pub fn get<V: SqValue>(&self, key: &str) -> Option<V::Item<'vm>> {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [table]
			self.vm.push_string(key); // [table, key]
			if squirrel_sys::sq_get(ptr, -2) != SQ_OK {
				squirrel_sys::sq_pop(ptr, 1); // sq_get popped the key; pop table
				return None;
			}
			// [table, value]
			let value = V::read(self.vm, -1);
			squirrel_sys::sq_pop(ptr, 2);
			value
		}
	}

	/// The value at `key` in THIS table only (no delegate fallthrough, like
	/// `t.rawget(key)`), read as `V`; `None` if absent or not a `V`.
	pub fn get_owned<V: SqValue>(&self, key: &str) -> Option<V::Item<'vm>> {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [table]
			self.vm.push_string(key); // [table, key]
			if squirrel_sys::sq_rawget(ptr, -2) != SQ_OK {
				squirrel_sys::sq_pop(ptr, 1); // sq_rawget popped the key; pop table
				return None;
			}
			// [table, value]
			let value = V::read(self.vm, -1);
			squirrel_sys::sq_pop(ptr, 2);
			value
		}
	}

	/// The value at `key` as a matchable [`SqRef`], or `None` if absent.
	pub fn get_ref(&self, key: &str) -> Option<SqRef<'vm>> {
		self.get::<SqRef>(key)
	}

	/// Whether `key` exists (following the delegate chain, like `get`).
	pub fn contains(&self, key: &str) -> bool {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [table]
			self.vm.push_string(key); // [table, key]
			if squirrel_sys::sq_get(ptr, -2) == SQ_OK {
				squirrel_sys::sq_pop(ptr, 2); // pop value + table
				true
			} else {
				squirrel_sys::sq_pop(ptr, 1); // sq_get popped the key; pop table
				false
			}
		}
	}

	/// Insert or overwrite the slot at `key` (an `SqArray`/`SqTable` value nests
	/// it by reference).
	pub fn set<V: SqValue>(&mut self, key: &str, value: V) {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [table]
			self.vm.push_string(key); // [table, key]
			value.push_value(self.vm); // [table, key, value]
			squirrel_sys::sq_newslot(ptr, -3, 0); // [table] (upsert; pops key + value)
			squirrel_sys::sq_pop(ptr, 1);
		}
	}

	/// Remove the slot at `key`. Returns `true` if it existed.
	pub fn remove(&mut self, key: &str) -> bool {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [table]
			self.vm.push_string(key); // [table, key]
			// `sq_deleteslot` pops the key on both success and failure -> [table].
			let existed = squirrel_sys::sq_deleteslot(ptr, -2, 0) == SQ_OK;
			squirrel_sys::sq_pop(ptr, 1);
			existed
		}
	}

	/// The string keys of this table (non-string keys are skipped). Order is
	/// unspecified.
	pub fn keys(&self) -> Vec<String> {
		let ptr = self.vm.as_ptr();
		let mut keys = Vec::new();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [table]
			let table_idx = squirrel_sys::sq_gettop(ptr); // absolute index
			squirrel_sys::sq_pushnull(ptr); // [table, iterator]
			while squirrel_sys::sq_next(ptr, table_idx) == SQ_OK {
				// [table, iterator, key, value]
				if let Ok(key) = self.vm.get_str_ref(-2) {
					keys.push(key.to_owned());
				}
				squirrel_sys::sq_pop(ptr, 2); // pop value + key -> [table, iterator]
			}
			squirrel_sys::sq_pop(ptr, 2); // pop iterator + table
		}
		keys
	}

	/// Every (key, value) pair as matchable [`SqRef`]s. Order is unspecified.
	/// Eager (collected into a `Vec`); one level only (nested tables/arrays come
	/// back as handles, not flattened).
	pub fn entries(&self) -> Vec<(SqRef<'vm>, SqRef<'vm>)> {
		let ptr = self.vm.as_ptr();
		let mut entries = Vec::new();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [table]
			let table_idx = squirrel_sys::sq_gettop(ptr);
			squirrel_sys::sq_pushnull(ptr); // [table, iterator]
			while squirrel_sys::sq_next(ptr, table_idx) == SQ_OK {
				// [table, iterator, key, value]
				let key = SqRef::read(self.vm, -2);
				let value = SqRef::read(self.vm, -1);
				squirrel_sys::sq_pop(ptr, 2); // pop value + key
				if let (Some(key), Some(value)) = (key, value) {
					entries.push((key, value));
				}
			}
			squirrel_sys::sq_pop(ptr, 2); // pop iterator + table
		}
		entries
	}

	/// A shallow copy: a new table with the same slots (nested tables/arrays are
	/// shared by reference, not deep-copied), and the same delegate.
	pub fn shallow_clone(&self) -> SqTable<'vm> {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [self]
			if squirrel_sys::sq_clone(ptr, -1) == SQ_OK {
				// [self, clone]
				let clone = SqTable::from_stack(self.vm, -1).expect("clone of a table is a table");
				squirrel_sys::sq_pop(ptr, 2);
				clone
			} else {
				// `sq_clone` popped its scratch null on failure -> [self]
				squirrel_sys::sq_pop(ptr, 1);
				SqTable::new(self.vm)
			}
		}
	}

	/// A deep copy: a new table whose nested tables/arrays are recursively cloned
	/// (so the copy shares no mutable container with the original); every other
	/// value (scalars, strings, closures, instances, …) is shared by reference.
	/// Unlike [`shallow_clone`](Self::shallow_clone), the delegate is *not*
	/// carried over. Assumes the table is acyclic (true of config / `m` data).
	pub fn deep_clone(&self) -> SqTable<'vm> {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [self]
			push_deep_clone(self.vm, -1); // [self, clone]
			let clone = SqTable::from_stack(self.vm, -1).expect("deep clone of a table is a table");
			squirrel_sys::sq_pop(ptr, 2);
			clone
		}
	}

	/// Overlay `src` onto this table (a deep merge): for each key in `src`, if
	/// both tables hold a sub-table there, merge it recursively; otherwise set the
	/// key to a deep clone of `src`'s value (overwriting any existing one). The
	/// merged-in values are deep-cloned, so the result shares no mutable container
	/// with `src`. Used to layer a child `m` table over its parent's defaults.
	pub fn deep_merge_from(&mut self, src: &SqTable) {
		let ptr = self.vm.as_ptr();
		for key in src.keys() {
			let both_tables = matches!(self.get_ref(&key), Some(SqRef::Table(_)))
				&& matches!(src.get_ref(&key), Some(SqRef::Table(_)));
			if both_tables {
				let mut dst_child = self.get::<SqTable>(&key).expect("table");
				let src_child = src.get::<SqTable>(&key).expect("table");
				dst_child.deep_merge_from(&src_child);
			} else {
				unsafe {
					squirrel_sys::sq_pushobject(ptr, self.obj); // [self]
					self.vm.push_string(&key); // [self, key]
					squirrel_sys::sq_pushobject(ptr, src.obj); // [self, key, src]
					self.vm.push_string(&key); // [self, key, src, key]
					if squirrel_sys::sq_get(ptr, -2) == SQ_OK {
						// [self, key, src, value]
						squirrel_sys::sq_remove(ptr, -2); // drop src -> [self, key, value]
						push_deep_clone(self.vm, -1); // [self, key, value, clone]
						squirrel_sys::sq_remove(ptr, -2); // drop original value -> [self, key, clone]
						squirrel_sys::sq_newslot(ptr, -3, 0); // pops key + clone -> [self]
						squirrel_sys::sq_pop(ptr, 1); // []
					} else {
						// `key` came from `src.keys()`, so this is unreachable.
						squirrel_sys::sq_pop(ptr, 3); // self, key, src (sq_get popped its key)
					}
				}
			}
		}
	}

	/// Set this table's delegate (its inheritance fallback): reads of a missing
	/// key fall through to `delegate`.
	pub fn set_delegate(&mut self, delegate: &SqTable) {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [self]
			squirrel_sys::sq_pushobject(ptr, delegate.obj); // [self, delegate]
			squirrel_sys::sq_setdelegate(ptr, -2); // sets self's delegate; pops delegate -> [self]
			squirrel_sys::sq_pop(ptr, 1);
		}
	}

	/// Remove this table's delegate (reads of a missing key no longer fall through).
	pub fn clear_delegate(&mut self) {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [self]
			squirrel_sys::sq_pushnull(ptr); // [self, null]
			squirrel_sys::sq_setdelegate(ptr, -2); // pops null -> [self]
			squirrel_sys::sq_pop(ptr, 1);
		}
	}

	/// This table's delegate, or `None` if it has none.
	pub fn delegate(&self) -> Option<SqTable<'vm>> {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [self]
			squirrel_sys::sq_getdelegate(ptr, -1); // [self, delegate-or-null]
			let delegate = SqTable::from_stack(self.vm, -1);
			squirrel_sys::sq_pop(ptr, 2);
			delegate
		}
	}

	/// Promote this borrowed handle to an owned [`SqStrongRef`] of the same table
	/// (takes an addref). Use it to return a built table from a native function.
	pub fn to_strong(&self) -> SqStrongRef {
		let ptr = self.vm.as_ptr();
		unsafe { squirrel_sys::sq_pushobject(ptr, self.obj) };
		let strong = SqStrongRef::from_stack(self.vm, -1);
		unsafe { squirrel_sys::sq_pop(ptr, 1) };
		strong
	}

	/// Call the method `name` on this table with `this` bound to the table itself,
	/// discarding any return value.
	///
	/// The lookup follows the delegate chain (like `this.name(...)` in a script),
	/// so an *inherited* method resolves — this is how the module system invokes a
	/// constructor (`create()`) that a class inherited from a parent. Arguments are
	/// passed as a tuple (`()`, `(a,)`, `(a, b)`, …).
	///
	/// Returns `Ok(false)` if no such method exists (e.g. a class without a
	/// `create`), `Ok(true)` if it ran, or the thrown error text if it raised.
	pub fn call_method<A: CallArgs>(&self, name: &str, args: A) -> Result<bool, String> {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [self]
			self.vm.push_string(name); // [self, name]
			if squirrel_sys::sq_get(ptr, -2) != SQ_OK {
				// sq_get popped the key; pop self. No such method.
				squirrel_sys::sq_pop(ptr, 1);
				return Ok(false);
			}
			// [self, method]; set up the call: closure, this, args..
			squirrel_sys::sq_pushobject(ptr, self.obj); // [self, method, this]
			let n = args.push_all(self.vm); // [self, method, this, args..]
			if squirrel_sys::sq_call(ptr, n + 1, 0, 1) == SQ_OK {
				// sq_call popped this + args; the method stays -> [self, method]
				squirrel_sys::sq_pop(ptr, 2); // pop method + self
				Ok(true)
			} else {
				// sq_call popped this + args -> [self, method]
				squirrel_sys::sq_getlasterror(ptr); // [self, method, error]
				let message = self
					.vm
					.get_str_ref(-1)
					.map(str::to_owned)
					.unwrap_or_else(|_| format!("method '{name}' raised a non-string error"));
				squirrel_sys::sq_pop(ptr, 1); // pop error
				squirrel_sys::sq_reseterror(ptr);
				squirrel_sys::sq_pop(ptr, 2); // pop method + self
				Err(message)
			}
		}
	}
}

/// Lets an `SqTable` be a *value* — read as a nested handle and pushed into
/// another container by reference.
impl<'vm> SqValue for SqTable<'vm> {
	type Item<'a> = SqTable<'a>;
	fn push_value(self, vm: &SQVM) {
		unsafe { squirrel_sys::sq_pushobject(vm.as_ptr(), self.obj) };
	}
	fn read<'a>(vm: &'a SQVM, idx: i64) -> Option<SqTable<'a>> {
		SqTable::from_stack(vm, idx)
	}
}

impl Drop for SqTable<'_> {
	fn drop(&mut self) {
		unsafe { squirrel_sys::sq_release(self.vm.as_ptr(), &mut self.obj) };
	}
}

// Non-recursive (a table can contain itself; a recursive Debug would loop).
impl std::fmt::Debug for SqTable<'_> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("SqTable").finish_non_exhaustive()
	}
}

#[cfg(test)]
mod tests {
	use super::SqTable;
	use crate::{SQVM, SqArray, SqRef};

	#[test]
	fn map_like_operations() {
		let vm = SQVM::new(1024);
		let mut t = SqTable::new(&vm);

		assert!(t.is_empty());
		t.set("hp", 50i64);
		t.set("name", "bob".to_string());
		t.set("alive", true);
		assert_eq!(t.len(), 3);

		assert_eq!(t.get::<i64>("hp"), Some(50));
		assert_eq!(t.get::<String>("name"), Some("bob".to_string()));
		assert_eq!(t.get::<bool>("alive"), Some(true));
		assert_eq!(t.get::<i64>("missing"), None);
		assert_eq!(t.get::<String>("hp"), None, "wrong type -> None");

		assert!(t.contains("hp"));
		assert!(!t.contains("missing"));

		t.set("hp", 75i64); // overwrite
		assert_eq!(t.get::<i64>("hp"), Some(75));

		assert!(t.remove("alive"));
		assert!(!t.remove("alive"), "second remove returns false");
		assert!(!t.contains("alive"));
		assert_eq!(t.len(), 2);

		let mut keys = t.keys();
		keys.sort();
		assert_eq!(keys, vec!["hp".to_string(), "name".to_string()]);
	}

	#[test]
	fn call_method_invokes_with_this_and_follows_delegates() {
		let vm = SQVM::new(1024);
		vm.run_script(
			r#"
			::Parent <- {
				function create() { this.made <- "by parent"; }
				function setHp(v) { this.hp <- v; }
				function boom() { throw "kaboom"; }
			};
			::Child <- {};
			"#,
			"setup",
		)
		.expect("setup");

		let parent = SqTable::root(&vm).get::<SqTable>("Parent").expect("Parent");
		let mut child = SqTable::root(&vm).get::<SqTable>("Child").expect("Child");
		child.set_delegate(&parent);

		// The inherited `create()` resolves via the delegate and runs with
		// `this == child`, so it writes onto the child.
		assert_eq!(child.call_method("create", ()), Ok(true));
		assert_eq!(child.get::<String>("made"), Some("by parent".to_string()));

		// A method with an argument.
		assert_eq!(child.call_method("setHp", (42i64,)), Ok(true));
		assert_eq!(child.get::<i64>("hp"), Some(42));

		// A missing method is `Ok(false)`, not an error (a class may lack `create`).
		assert_eq!(child.call_method("nope", ()), Ok(false));

		// A throwing method surfaces the error text and leaves the VM usable.
		let err = child.call_method("boom", ()).expect_err("should throw");
		assert!(err.contains("kaboom"), "error text: {err}");
		vm.run_script("::ok <- 1;", "after")
			.expect("vm still usable");
	}

	#[test]
	fn nested_handles() {
		let vm = SQVM::new(1024);
		let mut outer = SqTable::new(&vm);

		let mut inner = SqTable::new(&vm);
		inner.set("x", 7i64);
		outer.set("sub", inner);

		let mut arr = SqArray::new(&vm);
		arr.push(1i64);
		arr.push(2i64);
		outer.set("list", arr);

		// Chained borrowed reads into the nested table / array.
		assert_eq!(
			outer.get::<SqTable>("sub").unwrap().get::<i64>("x"),
			Some(7)
		);
		assert_eq!(outer.get::<SqArray>("list").unwrap().get::<i64>(1), Some(2));

		// Dynamic match, including the new Table variant.
		match outer.get_ref("sub") {
			Some(SqRef::Table(sub)) => assert_eq!(sub.get::<i64>("x"), Some(7)),
			other => panic!("expected Table, got {other:?}"),
		}
		assert!(matches!(outer.get_ref("list"), Some(SqRef::Array(_))));
	}

	#[test]
	fn nested_mutation_is_shared() {
		let vm = SQVM::new(1024);
		let mut outer = SqTable::new(&vm);
		let mut inner = SqTable::new(&vm);
		inner.set("n", 1i64);
		outer.set("sub", inner); // shared by reference

		// Mutate via a freshly-fetched handle...
		outer.get::<SqTable>("sub").unwrap().set("n", 99i64);
		// ...visible through the parent.
		assert_eq!(
			outer.get::<SqTable>("sub").unwrap().get::<i64>("n"),
			Some(99)
		);
	}

	#[test]
	fn deep_clone_is_independent() {
		let vm = SQVM::new(1024);
		let mut original = SqTable::new(&vm);
		original.set("scalar", 1i64);
		let mut nested = SqTable::new(&vm);
		nested.set("x", 10i64);
		original.set("nested", nested);
		let mut arr = SqArray::new(&vm);
		arr.push(7i64);
		original.set("arr", arr);

		let clone = original.deep_clone();
		assert_eq!(clone.get::<i64>("scalar"), Some(1));
		assert_eq!(
			clone.get::<SqTable>("nested").unwrap().get::<i64>("x"),
			Some(10)
		);
		assert_eq!(clone.get::<SqArray>("arr").unwrap().get::<i64>(0), Some(7));

		// Mutating the original's nested containers does not touch the clone.
		original.get::<SqTable>("nested").unwrap().set("x", 999i64);
		original.get::<SqArray>("arr").unwrap().set(0, 999i64);
		assert_eq!(
			clone.get::<SqTable>("nested").unwrap().get::<i64>("x"),
			Some(10),
			"nested table is independent"
		);
		assert_eq!(
			clone.get::<SqArray>("arr").unwrap().get::<i64>(0),
			Some(7),
			"nested array is independent"
		);
	}

	#[test]
	fn deep_merge_overlays_and_recurses() {
		let vm = SQVM::new(1024);
		let mut base = SqTable::new(&vm);
		base.set("keep", 1i64);
		base.set("override", 2i64);
		let mut base_sub = SqTable::new(&vm);
		base_sub.set("a", 10i64);
		base.set("sub", base_sub);

		let mut delta = SqTable::new(&vm);
		delta.set("override", 20i64);
		delta.set("added", 3i64);
		let mut delta_sub = SqTable::new(&vm);
		delta_sub.set("b", 30i64);
		delta.set("sub", delta_sub);

		base.deep_merge_from(&delta);

		assert_eq!(base.get::<i64>("keep"), Some(1), "untouched key kept");
		assert_eq!(base.get::<i64>("override"), Some(20), "overlaid");
		assert_eq!(base.get::<i64>("added"), Some(3), "new key added");
		// Sub-table merged recursively: `a` from base, `b` from delta.
		let sub = base.get::<SqTable>("sub").unwrap();
		assert_eq!(sub.get::<i64>("a"), Some(10));
		assert_eq!(sub.get::<i64>("b"), Some(30));

		// The merged-in value was deep-cloned, so it is independent of delta.
		delta.get::<SqTable>("sub").unwrap().set("b", 999i64);
		assert_eq!(
			base.get::<SqTable>("sub").unwrap().get::<i64>("b"),
			Some(30),
			"merged value was deep-cloned"
		);
	}

	#[test]
	fn entries_collects_pairs() {
		let vm = SQVM::new(1024);
		let mut t = SqTable::new(&vm);
		t.set("a", 1i64);
		t.set("b", 2i64);

		let mut pairs: Vec<(String, i64)> = t
			.entries()
			.into_iter()
			.filter_map(|(k, v)| match (k, v) {
				(SqRef::String(k), SqRef::Integer(v)) => Some((k, v)),
				_ => None,
			})
			.collect();
		pairs.sort();
		assert_eq!(pairs, vec![("a".to_string(), 1), ("b".to_string(), 2)]);
	}

	#[test]
	fn root_table_round_trips_with_squirrel() {
		let mut vm = SQVM::new(1024);

		// Rust -> Squirrel: write a global via the root table.
		{
			let mut globals = SqTable::root(&vm);
			let mut data = SqTable::new(&vm);
			data.set("level", 5i64);
			globals.set("Player", data);
		}

		// Script sees and mutates the same live table.
		vm.execute_squirrel_script(
			r#"
			assert(Player.level == 5);
			Player.level = 9;
			Player.gold <- 100;
			"#,
		)
		.expect("script sees the global table from Rust");

		// Squirrel -> Rust: read the script's changes back.
		let globals = SqTable::root(&vm);
		let player = globals.get::<SqTable>("Player").expect("Player table");
		assert_eq!(player.get::<i64>("level"), Some(9));
		assert_eq!(player.get::<i64>("gold"), Some(100));
	}

	#[test]
	fn shallow_clone_is_independent() {
		let vm = SQVM::new(1024);
		let mut original = SqTable::new(&vm);
		original.set("a", 1i64);

		let mut clone = original.shallow_clone();
		clone.set("a", 99i64); // overwrite a copied slot
		clone.set("b", 2i64); // add a new slot

		assert_eq!(original.get::<i64>("a"), Some(1), "original slot unchanged");
		assert!(
			!original.contains("b"),
			"clone's new slot is not in the original"
		);
		assert_eq!(clone.get::<i64>("a"), Some(99));
	}

	#[test]
	fn delegate_resolves_through_the_chain() {
		let mut vm = SQVM::new(1024);
		{
			let mut parent = SqTable::new(&vm);
			parent.set("inherited", 7i64);

			let mut child = SqTable::new(&vm);
			child.set("own", 1i64);
			child.set_delegate(&parent);
			assert!(child.delegate().is_some(), "delegate is set");

			// `get`/`contains` follow the delegate chain (sq_get does delegate
			// fallback), so an inherited key resolves from Rust too.
			assert_eq!(child.get::<i64>("own"), Some(1));
			assert_eq!(
				child.get::<i64>("inherited"),
				Some(7),
				"inherited via delegate"
			);
			assert!(child.contains("inherited"));

			SqTable::root(&vm).set("Child", child);
		}
		// And the same is true for script-side lookups.
		vm.execute_squirrel_script(
			r#"
			assert(Child.own == 1);
			assert(Child.inherited == 7);
			"#,
		)
		.expect("delegated key resolves in script");
	}
}
