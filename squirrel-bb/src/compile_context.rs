//! What a compile depends on besides the source text: the VM's constant table (the
//! compiler substitutes `const` and `enum` values) and whether it emits debug info.
//! A compile cache must key on this as well as on the source.

use squirrel::{SQVM, SqRef, SqTable};

/// A fingerprint of everything in this VM that changes what compiling a given source
/// produces (see the module docs). Independent of the constant table's iteration order.
pub(crate) fn compile_context(vm: &SQVM) -> u64 {
	mix(table_hash(&SqTable::consts(vm)) ^ u64::from(vm.debug_info_enabled()))
}

fn mix(mut h: u64) -> u64 {
	h ^= h >> 33;
	h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
	h ^= h >> 33;
	h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
	h ^ (h >> 33)
}

/// Order-independent: entries are combined by addition.
fn table_hash(table: &SqTable) -> u64 {
	table.entries().iter().fold(0x0074_6162_6c65, |acc, (k, v)| {
		acc.wrapping_add(mix(value_hash(k) ^ value_hash(v).rotate_left(17)))
	})
}

/// FNV-1a over a string's bytes.
fn bytes_hash(bytes: &[u8]) -> u64 {
	bytes
		.iter()
		.fold(5, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

fn value_hash(value: &SqRef) -> u64 {
	match value {
		SqRef::Null => mix(1),
		SqRef::Integer(n) => mix(2 ^ (*n as u64).rotate_left(8)),
		SqRef::Float(f) => mix(3 ^ f.to_bits().rotate_left(8)),
		SqRef::Bool(b) => mix(4 ^ (u64::from(*b) << 8)),
		SqRef::String(s) => mix(bytes_hash(s.as_bytes())),
		SqRef::Bytes(b) => mix(bytes_hash(b)),
		SqRef::Table(t) => mix(6 ^ table_hash(t)),
		SqRef::Array(a) => mix(a
			.to_vec::<SqRef>()
			.iter()
			.fold(7, |h, v| mix(h ^ value_hash(v)))),
		SqRef::Other(kind) => mix(8 ^ ((*kind as u64) << 8)),
		// Constants are scalars, strings or enum tables; no other kind can appear.
		_ => mix(9),
	}
}
