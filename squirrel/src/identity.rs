//! [`Identity`]: a native-fn argument yielding any object's stable identity hash.
//!
//! A stable integer that is unique per live object, which the object's address
//! satisfies for reference types (Squirrel never relocates a live object: it is plain
//! reference-counting, no compaction). Native method injection keys on it. (The
//! scripts' `toHash(x)` is [`ScriptHash`](crate::ScriptHash), the original's formula.)
//! So:
//!
//!   * reference types (instance/table/array/string/closure/…) → their pointer,
//!   * scalars (integer/bool) → the value, float → its bit pattern, null → 0.
//!
//! Interned strings share one [`SQString`](squirrel_sys::SQString), so equal
//! strings hash equally; identity is otherwise per-object, not per-value.

use crate::function::SqArg;
use crate::{SQ_OK, SQVM, SqObject};

/// A native-fn argument capturing the identity hash of the object passed at that
/// position. Use it as a parameter type to accept a value of *any* type and read
/// its identity — see [the module docs](self).
pub struct Identity(pub i64);

impl SqArg for Identity {
	type Item<'a> = Identity;

	fn push_typemask(mask: &mut String) {
		// Any type — `toHash` is called on instances, strings, and more.
		mask.push('.');
	}

	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<Identity, String> {
		let mut raw = std::mem::MaybeUninit::<squirrel_sys::SQObject>::uninit();
		// SAFETY: a valid stack index is handed in by the call machinery; on success
		// `sq_getstackobj` fully initializes the object.
		let res = unsafe { squirrel_sys::sq_getstackobj(vm.as_ptr(), idx, raw.as_mut_ptr()) };
		if res != SQ_OK {
			return Err("toHash: could not read the argument".to_string());
		}
		let obj = unsafe { SqObject::from_raw(raw.assume_init()) };
		Ok(Identity(obj.identity()))
	}
}

impl SQVM {
	/// The pointer-identity of the receiver (`this`, stack index 1) of the *current
	/// native call* — the same stable integer [`Identity`] yields, but for the
	/// receiver rather than an argument. Returns `0` if there is no valid receiver.
	///
	/// This is the abstraction native **method injection** uses to recover which
	/// script object a shared native method was invoked on (the state stack, and
	/// later the World/Tactical entity methods that index a Rust-owned arena). It
	/// returns a plain integer — no stack handle is exposed.
	pub fn this_identity(&self) -> i64 {
		let mut raw = std::mem::MaybeUninit::<squirrel_sys::SQObject>::uninit();
		if unsafe { squirrel_sys::sq_getstackobj(self.as_ptr(), 1, raw.as_mut_ptr()) } != SQ_OK {
			return 0;
		}
		// SAFETY: `sq_getstackobj` succeeded, so the object is initialized; `from_raw`
		// falls back gracefully only on a recognized tag (a malformed `_type` panics,
		// matching the rest of the crate's contract for a VM-produced object).
		unsafe { SqObject::from_raw(raw.assume_init()) }.identity()
	}

	/// The receiver (`this`) of the native call in progress, held: for a native that
	/// takes `&SQVM` and fixed arguments, where [`This`](crate::This) can't go.
	pub fn this_ref(&self) -> crate::SqStrongRef {
		crate::SqStrongRef::from_stack(self, 1)
	}
}

/// A map keyed by object identities ([`SQVM::this_identity`], [`Identity`]) or other
/// small integer keys (entity ids), with a hasher made for them: one multiply, its high
/// bits folded into the low ones (an identity is an aligned address, whose low bits are
/// all zero). The standard map's SipHash costs several times as much, on lookups every
/// injected native makes. Its iteration order is no more meaningful than the standard
/// map's (which is random per process).
pub type IdentityMap<V> = std::collections::HashMap<i64, V, std::hash::BuildHasherDefault<IdentityHasher>>;

/// The hasher of [`IdentityMap`].
#[derive(Default, Clone, Copy)]
pub struct IdentityHasher(u64);

impl std::hash::Hasher for IdentityHasher {
	fn write(&mut self, bytes: &[u8]) {
		// Only `i64` keys are expected; fold anything else in 8 bytes at a time.
		for chunk in bytes.chunks(8) {
			let mut word = [0u8; 8];
			word[..chunk.len()].copy_from_slice(chunk);
			self.write_u64(u64::from_le_bytes(word));
		}
	}

	fn write_u64(&mut self, n: u64) {
		let h = (self.0 ^ n).wrapping_mul(0x9E37_79B9_7F4A_7C15);
		self.0 = h ^ (h >> 32);
	}

	fn write_i64(&mut self, n: i64) {
		self.write_u64(n as u64);
	}

	fn finish(&self) -> u64 {
		self.0
	}
}

#[cfg(test)]
mod map_tests {
	use super::IdentityMap;

	#[test]
	fn aligned_keys_spread_over_the_buckets() {
		use std::hash::BuildHasher;
		let build = std::hash::BuildHasherDefault::<super::IdentityHasher>::default();
		// Addresses 16 apart, as the VM's objects are: the low 6 bits of their hashes
		// (a 64-bucket table's index) take every value.
		let mut seen = [false; 64];
		for i in 0..4096i64 {
			seen[(build.hash_one(0x5555_0000_0000 + i * 16) & 63) as usize] = true;
		}
		assert!(seen.iter().all(|&s| s));
		let mut map = IdentityMap::default();
		map.insert(7, "a");
		map.insert(-3, "b");
		assert_eq!(map.get(&7), Some(&"a"));
		assert_eq!(map.get(&-3), Some(&"b"));
		assert_eq!(map.get(&8), None);
	}
}
