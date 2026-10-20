//! The tables that make a font variable along the size axis: `fvar` (the axis),
//! `HVAR` (what each glyph's advance adds along it) and the axis's name in `name`.

use super::sfnt::u16_at;
use super::sizes::{AXIS_MAX, Region, deltas, regions, word_count};

/// `fvar` with one hidden `opsz` axis, 0 to [`AXIS_MAX`] (default 0), named `name_id`.
pub fn fvar(name_id: u16) -> Vec<u8> {
	let mut out = Vec::new();
	for word in [1u16, 0, 16, 2, 1, 20, 0, 8] {
		out.extend_from_slice(&word.to_be_bytes());
	}
	out.extend_from_slice(b"opsz");
	for fixed in [0u32, 0, (AXIS_MAX as u32) << 16] {
		out.extend_from_slice(&fixed.to_be_bytes());
	}
	// Hidden: no font menu or picker should offer it.
	out.extend_from_slice(&1u16.to_be_bytes());
	out.extend_from_slice(&name_id.to_be_bytes());
	out
}

/// `HVAR` for glyphs advancing `advances` units (one per glyph) in a font of
/// `units_per_em`: one delta set per distinct advance, mapped from each glyph.
pub fn hvar(advances: &[u16], units_per_em: u16) -> Option<Vec<u8>> {
	let mut distinct: Vec<u16> = advances.to_vec();
	distinct.sort_unstable();
	distinct.dedup();
	if distinct.len() > u16::MAX as usize || advances.len() > u16::MAX as usize {
		return None;
	}
	let store = variation_store(&regions(), &distinct, units_per_em);

	// The glyphs' delta-set indices (outer 0, inner the advance's index).
	let bits = (usize::BITS - distinct.len().saturating_sub(1).leading_zeros()).max(1);
	let entry_size = if bits <= 8 { 1 } else { 2 };
	let mut map = vec![0u8, (((entry_size - 1) << 4) | (bits - 1)) as u8];
	map.extend_from_slice(&(advances.len() as u16).to_be_bytes());
	for advance in advances {
		let index = distinct.binary_search(advance).ok()? as u16;
		map.extend_from_slice(&index.to_be_bytes()[2 - entry_size as usize..]);
	}

	let mut out = Vec::new();
	out.extend_from_slice(&1u16.to_be_bytes());
	out.extend_from_slice(&0u16.to_be_bytes());
	let store_at = 20u32;
	let map_at = store_at + store.len() as u32;
	for offset in [store_at, map_at, 0, 0] {
		out.extend_from_slice(&offset.to_be_bytes());
	}
	out.extend_from_slice(&store);
	out.extend_from_slice(&map);
	Some(out)
}

/// An ItemVariationStore over `regions`, one delta set per advance in `advances`.
fn variation_store(regions: &[Region], advances: &[u16], units_per_em: u16) -> Vec<u8> {
	let mut list = Vec::new();
	list.extend_from_slice(&1u16.to_be_bytes());
	list.extend_from_slice(&(regions.len() as u16).to_be_bytes());
	for &(start, peak, end) in regions {
		for coordinate in [start, peak, end] {
			list.extend_from_slice(&(coordinate.min(16384) as i16).to_be_bytes());
		}
	}

	let words = word_count();
	let mut data = Vec::new();
	data.extend_from_slice(&(advances.len() as u16).to_be_bytes());
	data.extend_from_slice(&(words as u16).to_be_bytes());
	data.extend_from_slice(&(regions.len() as u16).to_be_bytes());
	for index in 0..regions.len() {
		data.extend_from_slice(&(index as u16).to_be_bytes());
	}
	for &advance in advances {
		let (words, bytes) = deltas(advance, units_per_em);
		for delta in words {
			data.extend_from_slice(&delta.to_be_bytes());
		}
		data.extend(bytes.iter().map(|&d| d as u8));
	}

	let mut out = Vec::new();
	out.extend_from_slice(&1u16.to_be_bytes());
	let list_at = 2 + 4 + 2 + 4;
	out.extend_from_slice(&(list_at as u32).to_be_bytes());
	out.extend_from_slice(&1u16.to_be_bytes());
	out.extend_from_slice(&((list_at + list.len()) as u32).to_be_bytes());
	out.extend_from_slice(&list);
	out.extend_from_slice(&data);
	out
}

/// `name` with `text` added as an English (Windows, Unicode BMP) name, and the new
/// name's id (the first free one from 256).
pub fn add_name(name: &[u8], text: &str) -> Option<(Vec<u8>, u16)> {
	let records = name_records(name)?;
	let id = (256..=u16::MAX).find(|id| records.iter().all(|r| r[3] != *id))?;
	Some((set_names(name, &[(id, text)])?, id))
}

/// `name` with each `(id, text)` of `names` as the font's only name `id`: an English
/// (Windows, Unicode BMP) one, in place of any on every platform.
pub fn set_names(name: &[u8], names: &[(u16, &str)]) -> Option<Vec<u8>> {
	let format = u16_at(name, 0)?;
	if format > 1 {
		return None;
	}
	let mut records = name_records(name)?;
	let storage_at = u16_at(name, 4)? as usize;
	// Format 1's language tags sit between the records and the strings.
	let between = name.get(6 + records.len() * 12..storage_at)?;
	let mut storage = name.get(storage_at..)?.to_vec();
	records.retain(|r| names.iter().all(|(id, _)| r[3] != *id));
	for (id, text) in names {
		let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
		let at = u16::try_from(storage.len()).ok()?;
		records.push([3, 1, 0x409, *id, u16::try_from(bytes.len()).ok()?, at]);
		storage.extend_from_slice(&bytes);
	}
	records.sort_by_key(|r| (r[0], r[1], r[2], r[3]));

	let mut out = Vec::new();
	out.extend_from_slice(&format.to_be_bytes());
	out.extend_from_slice(&(records.len() as u16).to_be_bytes());
	out.extend_from_slice(&u16::try_from(6 + records.len() * 12 + between.len()).ok()?.to_be_bytes());
	for record in &records {
		for field in record {
			out.extend_from_slice(&field.to_be_bytes());
		}
	}
	out.extend_from_slice(between);
	out.extend_from_slice(&storage);
	Some(out)
}

/// The name records of `name`: platform, encoding, language, id, length, offset.
fn name_records(name: &[u8]) -> Option<Vec<[u16; 6]>> {
	let count = u16_at(name, 2)? as usize;
	(0..count)
		.map(|i| {
			let mut record = [0; 6];
			for (j, field) in record.iter_mut().enumerate() {
				*field = u16_at(name, 6 + i * 12 + j * 2)?;
			}
			Some(record)
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::super::sfnt::u16_at;
	use super::{add_name, fvar, hvar, set_names};

	#[test]
	fn names_the_axis_with_a_free_id() {
		// Two records: (3,1,0x409,1) "A" and (3,1,0x409,256) "B".
		let mut name = vec![0, 0, 0, 2, 0, 30];
		for (id, at) in [(1u16, 0u16), (256, 2)] {
			for field in [3u16, 1, 0x409, id, 2, at] {
				name.extend_from_slice(&field.to_be_bytes());
			}
		}
		name.extend_from_slice(&[0, b'A', 0, b'B']);
		let (out, id) = add_name(&name, "Size").unwrap();
		assert_eq!(id, 257);
		assert_eq!(u16_at(&out, 2), Some(3));
		assert_eq!(u16_at(&out, 4), Some(42));
		assert_eq!(&out[42..], &[0, b'A', 0, b'B', 0, b'S', 0, b'i', 0, b'z', 0, b'e']);
		// The new record is last (sorted), pointing past the old strings.
		assert_eq!(&out[30..42], &[0, 3, 0, 1, 4, 9, 1, 1, 0, 8, 0, 4]);
	}

	#[test]
	fn renames_on_every_platform() {
		// (1,0,0,1) "A" and (3,1,0x409,1) "A", (3,1,0x409,2) "B".
		let mut name = vec![0, 0, 0, 3, 0, 42];
		for (platform, encoding, language, id, at) in [(1u16, 0u16, 0u16, 1u16, 0u16), (3, 1, 0x409, 1, 0), (3, 1, 0x409, 2, 2)] {
			for field in [platform, encoding, language, id, 2, at] {
				name.extend_from_slice(&field.to_be_bytes());
			}
		}
		name.extend_from_slice(&[0, b'A', 0, b'B']);
		let out = set_names(&name, &[(1, "Z")]).unwrap();
		assert_eq!(u16_at(&out, 2), Some(2));
		assert_eq!(u16_at(&out, 4), Some(30));
		// (3,1,0x409,1) "Z" (after the old strings), then (3,1,0x409,2) "B".
		assert_eq!(&out[6..30], &[0, 3, 0, 1, 4, 9, 0, 1, 0, 2, 0, 4, 0, 3, 0, 1, 4, 9, 0, 2, 0, 2, 0, 2]);
		assert_eq!(&out[30..], &[0, b'A', 0, b'B', 0, b'Z']);
	}

	#[test]
	fn fvar_has_one_hidden_size_axis() {
		let table = fvar(300);
		assert_eq!(table.len(), 16 + 20);
		assert_eq!(&table[16..20], b"opsz");
		assert_eq!(u16_at(&table, 32), Some(1));
		assert_eq!(u16_at(&table, 34), Some(300));
	}

	#[test]
	fn hvar_maps_each_glyph_to_its_advance() {
		let table = hvar(&[500, 600, 500], 1000).unwrap();
		let map_at = u32::from_be_bytes(table[8..12].try_into().unwrap()) as usize;
		// Format 0, one-byte entries of one bit, three glyphs: 0, 1, 0.
		assert_eq!(&table[map_at..], &[0, 0, 0, 3, 0, 1, 0]);
	}
}
