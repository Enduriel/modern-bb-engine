//! [`SqArray`]: a `Vec`-like handle over a live Squirrel array.

use std::ffi::c_char;

use super::{SqRef, SqValue};
use crate::{SQ_OK, SQVM, SqObjectType, SqStrongRef};

/// A borrowed, `Vec`-like handle to a live Squirrel array.
///
/// Operations mutate the *actual* Squirrel array, so changes are visible to
/// scripts (and script-side changes are visible here). The handle keeps the
/// array alive with an `addref` and releases it on `Drop`; the `'vm` lifetime
/// ties it to the borrow of the [`SQVM`], so — like [`This`](crate::This) and
/// borrowed `&str` args — it cannot outlive the VM or be stored past that borrow.
///
/// Reads are by lifetime-parameterized [`SqValue`]: `get::<i64>` yields an owned
/// `i64`, while `get::<SqArray>` yields a *nested* `SqArray` handle into the same
/// live array (a reference, not a copy — mutating it is visible in the parent).
/// [`get_ref`](SqArray::get_ref) reads a value without knowing its type, as a
/// matchable [`SqRef`]. Reads return one level only, which keeps cyclic arrays
/// (an array containing itself) from looping — the caller drives any recursion.
///
/// ```ignore
/// let mut arr = SqArray::new(&vm);
/// arr.push(1i64);
/// let mut inner = SqArray::new(&vm);
/// inner.push("hi".to_string());
/// arr.push(inner);                       // nest an array (shared by reference)
/// assert_eq!(arr.get::<SqArray>(1).unwrap().get::<String>(0).as_deref(), Some("hi"));
/// match arr.get_ref(0).unwrap() {
///     SqRef::Integer(n) => { /* ... */ }
///     _ => {}
/// }
/// ```
pub struct SqArray<'vm> {
	vm: &'vm SQVM,
	obj: squirrel_sys::HSQOBJECT,
}

impl<'vm> SqArray<'vm> {
	/// Create a new, empty Squirrel array.
	pub fn new(vm: &'vm SQVM) -> Self {
		let ptr = vm.as_ptr();
		let mut obj: squirrel_sys::HSQOBJECT = unsafe { std::mem::zeroed() };
		unsafe {
			squirrel_sys::sq_newarray(ptr, 0); // [array]
			squirrel_sys::sq_getstackobj(ptr, -1, &mut obj);
			squirrel_sys::sq_addref(ptr, &mut obj); // keep it alive beyond the stack
			squirrel_sys::sq_pop(ptr, 1); // []
		}
		SqArray { vm, obj }
	}

	/// Wrap the array value at stack `idx` as an owned handle (addref'd), or
	/// `None` if that slot is not an array. Does not change the stack.
	pub(crate) fn from_stack(vm: &'vm SQVM, idx: i64) -> Option<Self> {
		if !matches!(vm.get_type(idx), SqObjectType::Array) {
			return None;
		}
		let mut obj: squirrel_sys::HSQOBJECT = unsafe { std::mem::zeroed() };
		unsafe {
			squirrel_sys::sq_getstackobj(vm.as_ptr(), idx, &mut obj);
			squirrel_sys::sq_addref(vm.as_ptr(), &mut obj);
		}
		Some(SqArray { vm, obj })
	}

	/// Wrap the array stored in the root table under `name`, or `None` if that
	/// slot is missing or not an array.
	pub fn from_global(vm: &'vm SQVM, name: &str) -> Option<Self> {
		let ptr = vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushroottable(ptr); // [root]
			squirrel_sys::sq_pushstring(ptr, name.as_ptr() as *const c_char, name.len() as i64); // [root, name]
			if squirrel_sys::sq_get(ptr, -2) != SQ_OK {
				// `sq_get` pops the key on failure.
				squirrel_sys::sq_pop(ptr, 1); // pop root
				return None;
			}
			// [root, value]
			let array = SqArray::from_stack(vm, -1);
			squirrel_sys::sq_pop(ptr, 2); // pop value + root
			array
		}
	}

	/// Bind this array into the root table under `name`, making it reachable from
	/// scripts. The array is shared, not copied: later mutations stay visible.
	pub fn set_global(&self, name: &str) {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushroottable(ptr); // [root]
			squirrel_sys::sq_pushstring(ptr, name.as_ptr() as *const c_char, name.len() as i64); // [root, name]
			squirrel_sys::sq_pushobject(ptr, self.obj); // [root, name, array]
			squirrel_sys::sq_newslot(ptr, -3, 0); // [root] (pops name + array)
			squirrel_sys::sq_pop(ptr, 1); // []
		}
	}

	/// Number of elements.
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

	/// The element at `index` read as `T`, or `None` if out of range or not a
	/// `T`. For a borrowed element type (e.g. `SqArray`) the returned handle
	/// borrows the VM for `'vm`, not just the `&self` call.
	pub fn get<T: SqValue>(&self, index: usize) -> Option<T::Item<'vm>> {
		if index >= self.len() {
			return None;
		}
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [arr]
			squirrel_sys::sq_pushinteger(ptr, index as i64); // [arr, idx]
			if squirrel_sys::sq_get(ptr, -2) != SQ_OK {
				squirrel_sys::sq_pop(ptr, 1); // sq_get popped the key; pop arr
				return None;
			}
			// [arr, value]
			let value = T::read(self.vm, -1);
			squirrel_sys::sq_pop(ptr, 2);
			value
		}
	}

	/// The element at `index` as a matchable [`SqRef`], or `None` if out of range.
	pub fn get_ref(&self, index: usize) -> Option<SqRef<'vm>> {
		self.get::<SqRef>(index)
	}

	/// Overwrite the element at `index`. Returns `false` if out of range.
	pub fn set<T: SqValue>(&mut self, index: usize, value: T) -> bool {
		if index >= self.len() {
			return false;
		}
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [arr]
			squirrel_sys::sq_pushinteger(ptr, index as i64); // [arr, idx]
			value.push_value(self.vm); // [arr, idx, value]
			if squirrel_sys::sq_set(ptr, -3) == SQ_OK {
				squirrel_sys::sq_pop(ptr, 1); // sq_set popped key + value; pop arr
				true
			} else {
				squirrel_sys::sq_pop(ptr, 3); // pop value, idx, arr
				false
			}
		}
	}

	/// Append `value` to the end. An `SqArray` value nests it by reference.
	pub fn push<T: SqValue>(&mut self, value: T) {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [arr]
			value.push_value(self.vm); // [arr, value]
			squirrel_sys::sq_arrayappend(ptr, -2); // [arr] (appends + pops value)
			squirrel_sys::sq_pop(ptr, 1);
		}
	}

	/// Remove and return the last element, or `None` if empty (or not a `T`).
	pub fn pop<T: SqValue>(&mut self) -> Option<T::Item<'vm>> {
		if self.is_empty() {
			return None;
		}
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [arr]
			if squirrel_sys::sq_arraypop(ptr, -1, 1) != SQ_OK {
				squirrel_sys::sq_pop(ptr, 1);
				return None;
			}
			// [arr, value]
			let value = T::read(self.vm, -1);
			squirrel_sys::sq_pop(ptr, 2);
			value
		}
	}

	/// Insert `value` so it ends up at `index`. Returns `false` if `index > len`.
	pub fn insert<T: SqValue>(&mut self, index: usize, value: T) -> bool {
		if index > self.len() {
			return false;
		}
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [arr]
			value.push_value(self.vm); // [arr, value]
			squirrel_sys::sq_arrayinsert(ptr, -2, index as i64); // [arr] (pops value)
			squirrel_sys::sq_pop(ptr, 1);
		}
		true
	}

	/// Remove the element at `index`, returning it as `T` if it was a `T`. `None`
	/// if out of range.
	pub fn remove<T: SqValue>(&mut self, index: usize) -> Option<T::Item<'vm>> {
		if index >= self.len() {
			return None;
		}
		let value = self.get::<T>(index);
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj);
			squirrel_sys::sq_arrayremove(ptr, -1, index as i64);
			squirrel_sys::sq_pop(ptr, 1);
		}
		value
	}

	/// Remove all elements.
	pub fn clear(&mut self) {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj);
			squirrel_sys::sq_arrayresize(ptr, -1, 0);
			squirrel_sys::sq_pop(ptr, 1);
		}
	}

	/// Reverse the elements in place.
	pub fn reverse(&mut self) {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj);
			squirrel_sys::sq_arrayreverse(ptr, -1);
			squirrel_sys::sq_pop(ptr, 1);
		}
	}

	/// Append every item of `iter`.
	pub fn extend<T: SqValue, I: IntoIterator<Item = T>>(&mut self, iter: I) {
		for value in iter {
			self.push(value);
		}
	}

	/// Iterate the elements read as `T`, skipping any that are not a `T`. One
	/// level only — a nested array yields a handle, it is not flattened.
	pub fn iter<T: SqValue>(&self) -> impl Iterator<Item = T::Item<'vm>> + '_ {
		(0..self.len()).filter_map(move |i| self.get::<T>(i))
	}

	/// Collect the elements read as `T`, skipping any that are not a `T`.
	pub fn to_vec<T: SqValue>(&self) -> Vec<T::Item<'vm>> {
		self.iter::<T>().collect()
	}

	/// Promote this borrowed handle to an owned [`SqStrongRef`] of the same array
	/// (takes an addref). Use it to return a built array from a native function.
	pub fn to_strong(&self) -> SqStrongRef {
		let ptr = self.vm.as_ptr();
		unsafe { squirrel_sys::sq_pushobject(ptr, self.obj) };
		let strong = SqStrongRef::from_stack(self.vm, -1);
		unsafe { squirrel_sys::sq_pop(ptr, 1) };
		strong
	}

	/// A deep copy: a new array whose nested tables/arrays are recursively cloned
	/// (so the copy shares no mutable container with the original); every other
	/// element is shared by reference. Assumes the array is acyclic.
	pub fn deep_clone(&self) -> SqArray<'vm> {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [self]
			super::deep::push_deep_clone(self.vm, -1); // [self, clone]
			let clone =
				SqArray::from_stack(self.vm, -1).expect("deep clone of an array is an array");
			squirrel_sys::sq_pop(ptr, 2);
			clone
		}
	}
}

/// Lets an `SqArray` be a *value* — read from (a nested array handle) and pushed
/// into (nesting it by reference) another container.
impl<'vm> SqValue for SqArray<'vm> {
	type Item<'a> = SqArray<'a>;
	fn push_value(self, vm: &SQVM) {
		// Push a reference to the same array. `self` is dropped (releasing our
		// addref) when this returns, but the stack now holds a reference, so the
		// array stays alive until the caller appends/sets it (which addrefs it
		// into the destination).
		unsafe { squirrel_sys::sq_pushobject(vm.as_ptr(), self.obj) };
	}
	fn read<'a>(vm: &'a SQVM, idx: i64) -> Option<SqArray<'a>> {
		SqArray::from_stack(vm, idx)
	}
}

impl Drop for SqArray<'_> {
	fn drop(&mut self) {
		unsafe { squirrel_sys::sq_release(self.vm.as_ptr(), &mut self.obj) };
	}
}

// Deliberately does NOT print elements: a cyclic array (one containing itself)
// would make a recursive `Debug` loop forever.
impl std::fmt::Debug for SqArray<'_> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("SqArray").finish_non_exhaustive()
	}
}

#[cfg(test)]
mod tests {
	use super::SqArray;
	use crate::{SQVM, SqObjectType, SqRef};

	#[test]
	fn vec_like_operations() {
		let vm = SQVM::new(1024);
		let mut arr = SqArray::new(&vm);

		assert!(arr.is_empty());
		arr.push(1i64);
		arr.push(2i64);
		arr.push(3i64);
		assert_eq!(arr.len(), 3);
		assert_eq!(arr.get::<i64>(1), Some(2));
		assert_eq!(arr.get::<i64>(3), None, "out of range");

		assert!(arr.set(1, 20i64));
		assert_eq!(arr.get::<i64>(1), Some(20));
		assert!(!arr.set(9, 0i64), "out-of-range set is rejected");

		assert_eq!(arr.pop::<i64>(), Some(3));
		assert_eq!(arr.len(), 2);

		assert!(arr.insert(0, 0i64));
		assert_eq!(arr.to_vec::<i64>(), vec![0, 1, 20]);
		assert!(!arr.insert(99, 0i64), "out-of-range insert is rejected");

		arr.reverse();
		assert_eq!(arr.to_vec::<i64>(), vec![20, 1, 0]);

		assert_eq!(arr.remove::<i64>(0), Some(20));
		assert_eq!(arr.to_vec::<i64>(), vec![1, 0]);

		arr.clear();
		assert!(arr.is_empty());
		assert_eq!(arr.pop::<i64>(), None, "pop on empty");
	}

	#[test]
	fn string_and_mixed_elements() {
		let vm = SQVM::new(1024);
		let mut arr = SqArray::new(&vm);

		arr.extend(["a".to_string(), "bc".to_string()]);
		assert_eq!(arr.get::<String>(1), Some("bc".to_string()));
		assert_eq!(
			arr.to_vec::<String>(),
			vec!["a".to_string(), "bc".to_string()]
		);

		// A wrong requested type yields `None` rather than a bogus value.
		let mut nums = SqArray::new(&vm);
		nums.push(42i64);
		assert_eq!(nums.get::<String>(0), None);
		assert_eq!(nums.get::<i64>(0), Some(42));
	}

	#[test]
	fn nested_arrays() {
		let vm = SQVM::new(1024);
		let mut outer = SqArray::new(&vm);
		outer.push(1i64);

		let mut inner = SqArray::new(&vm);
		inner.push(10i64);
		inner.push("hi".to_string());
		outer.push(inner); // `inner` moved in; the array now holds the reference

		assert_eq!(outer.len(), 2);
		// Chained borrowed read down into the nested array.
		let child = outer.get::<SqArray>(1).expect("element 1 is an array");
		assert_eq!(child.len(), 2);
		assert_eq!(child.get::<i64>(0), Some(10));
		assert_eq!(child.get::<String>(1), Some("hi".to_string()));
		// A scalar element is not an array.
		assert!(outer.get::<SqArray>(0).is_none());

		// insert an array at the front, too.
		let mut head = SqArray::new(&vm);
		head.push(7i64);
		assert!(outer.insert(0, head));
		assert_eq!(outer.get::<SqArray>(0).unwrap().get::<i64>(0), Some(7));
	}

	#[test]
	fn nested_mutation_is_shared() {
		let vm = SQVM::new(1024);
		let mut outer = SqArray::new(&vm);
		let mut inner = SqArray::new(&vm);
		inner.push(1i64);
		outer.push(inner); // shared by reference, not copied

		// Mutate through a freshly-fetched child handle...
		{
			let mut child = outer.get::<SqArray>(0).unwrap();
			child.push(2i64);
			child.set(0, 99i64);
		}
		// ...and observe it through the parent: same live object.
		let child = outer.get::<SqArray>(0).unwrap();
		assert_eq!(child.to_vec::<i64>(), vec![99, 2]);
	}

	#[test]
	fn dropped_inner_stays_alive_via_parent() {
		// The inner handle is consumed (and dropped, releasing our addref) by
		// `push`; the parent's append must have addref'd it first, so it survives.
		let vm = SQVM::new(1024);
		let mut outer = SqArray::new(&vm);
		{
			let mut inner = SqArray::new(&vm);
			inner.push(123i64);
			outer.push(inner); // inner dropped here
		}
		let child = outer
			.get::<SqArray>(0)
			.expect("inner survived its handle drop");
		assert_eq!(child.get::<i64>(0), Some(123));
	}

	#[test]
	fn get_ref_matches_on_type() {
		let vm = SQVM::new(1024);
		let mut arr = SqArray::new(&vm);
		arr.push(42i64);
		arr.push("text".to_string());
		arr.push(true);
		let mut inner = SqArray::new(&vm);
		inner.push(1i64);
		arr.push(inner);

		assert!(matches!(arr.get_ref(0), Some(SqRef::Integer(42))));
		assert!(matches!(arr.get_ref(2), Some(SqRef::Bool(true))));
		match arr.get_ref(1) {
			Some(SqRef::String(s)) => assert_eq!(s, "text"),
			other => panic!("expected String, got {other:?}"),
		}
		match arr.get_ref(3) {
			Some(SqRef::Array(a)) => assert_eq!(a.get::<i64>(0), Some(1)),
			other => panic!("expected Array, got {other:?}"),
		}
		assert!(arr.get_ref(99).is_none());
	}

	#[test]
	fn strings_that_are_not_utf8_read_as_bytes() {
		let vm = SQVM::new(1024);
		let mut arr = SqArray::new(&vm);
		arr.push(SqRef::Bytes(vec![0x61, 0xE9]));
		arr.push(SqRef::Bytes(vec![0xC3, 0xA9]));
		match arr.get_ref(0) {
			Some(SqRef::Bytes(b)) => assert_eq!(b, [0x61, 0xE9]),
			other => panic!("expected Bytes, got {other:?}"),
		}
		// Bytes that happen to be UTF-8 read as a string.
		match arr.get_ref(1) {
			Some(SqRef::String(s)) => assert_eq!(s, "é"),
			other => panic!("expected String, got {other:?}"),
		}
	}

	#[test]
	#[should_panic(expected = "SqRef::Other")]
	fn pushing_other_is_unimplemented() {
		let vm = SQVM::new(1024);
		let mut arr = SqArray::new(&vm);
		// `Other` carries only a type tag; pushing it is not yet supported.
		arr.push(SqRef::Other(SqObjectType::Table));
	}

	#[test]
	fn shared_with_squirrel_both_directions() {
		let mut vm = SQVM::new(1024);

		// Rust -> Squirrel: build an array and expose it as a global.
		{
			let mut arr = SqArray::new(&vm);
			arr.push(10i64);
			arr.push(20i64);
			arr.push(30i64);
			arr.set_global("nums");
		} // drop the handle; the root table keeps the array alive

		// The script sees the same live array, and mutates it.
		vm.execute_squirrel_script(
			r#"
			assert(nums.len() == 3);
			assert(nums[0] == 10 && nums[2] == 30);
			nums.append(40);
			nums[1] = 99;
			"#,
		)
		.expect("script should see the array bound from Rust");

		// Squirrel -> Rust: re-wrap and observe the script's changes.
		let arr = SqArray::from_global(&vm, "nums").expect("`nums` global array");
		assert_eq!(arr.len(), 4);
		assert_eq!(arr.to_vec::<i64>(), vec![10, 99, 30, 40]);

		assert!(
			SqArray::from_global(&vm, "missing").is_none(),
			"absent global -> None"
		);
	}
}
