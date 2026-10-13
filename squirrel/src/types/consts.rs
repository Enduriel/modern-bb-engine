//! The VM's constant table: what `const` and `enum` declarations add to, and what the
//! compiler substitutes from when it compiles later scripts.

use super::SqTable;
use crate::SQVM;

impl<'vm> SqTable<'vm> {
	/// The VM's constant table (`sq_pushconsttable`).
	pub fn consts(vm: &'vm SQVM) -> Self {
		let ptr = vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushconsttable(ptr); // [consts]
			let table = SqTable::from_stack(vm, -1);
			squirrel_sys::sq_pop(ptr, 1);
			table.expect("the constant table is a table")
		}
	}
}
