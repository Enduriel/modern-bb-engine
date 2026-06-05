//! The BB `toHash(obj)` root global.
//!
//! Returns a stable integer identifier for any object — the pointer for reference
//! types (instances, tables, …), the value for scalars. `item.getInstanceID()`
//! uses `toHash(this).tostring()` to key an item by identity; mods use it similarly.
//! The identity logic lives in [`squirrel::Identity`].

use squirrel::{Identity, SQVM};

/// Register the `toHash` root global.
pub(crate) fn install(vm: &mut SQVM) {
	vm.register("toHash", |x: Identity| x.0);
}
