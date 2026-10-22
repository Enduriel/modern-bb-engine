//! `bb.exe`'s icon as a compiled resource file (`.res`), which `build.rs` writes and
//! hands to the linker (link.exe and lld-link take `.res` files as inputs), so no
//! resource compiler is needed, on Windows or cross-building. Explorer and the taskbar
//! show the program's first icon group; the window takes it too (`super::window_icon`).
//!
//! A `.res` file is a run of resources, each a `RESOURCEHEADER` and its data, each
//! aligned to 4 bytes, after an empty first one that marks the format (32-bit). The
//! icon is one `RT_ICON` per image, as the `.ico` stores it, and an `RT_GROUP_ICON`
//! that lists them: the `.ico`'s directory with each image's resource id in place of
//! its offset.

use super::ico::Image;

pub const RT_ICON: u16 = 3;
pub const RT_GROUP_ICON: u16 = 14;
/// The icon group's id, which `LoadImage` names it by.
pub const GROUP_ID: u16 = 1;
/// The resources' language: English (United States), as resource compilers default to.
const LANGUAGE: u16 = 0x0409;
/// `MemoryFlags`: moveable, pure (and discardable for the group), as rc.exe writes them.
const ICON_FLAGS: u16 = 0x1010;
const GROUP_FLAGS: u16 = 0x1030;

/// The `.res` file with `images` as the icon group [`GROUP_ID`], their resource ids
/// counting from 1.
pub fn icon_res(images: &[Image]) -> Vec<u8> {
	let mut out = Vec::new();
	resource(&mut out, 0, 0, 0, &[]);
	let mut group = Vec::new();
	group.extend_from_slice(&[0, 0, 1, 0]);
	group.extend_from_slice(&(images.len() as u16).to_le_bytes());
	for (i, image) in images.iter().enumerate() {
		let id = i as u16 + 1;
		resource(&mut out, RT_ICON, id, ICON_FLAGS, image.data);
		let side = if image.size >= 256 { 0 } else { image.size as u8 };
		group.extend_from_slice(&[side, side, image.colors, 0]);
		group.extend_from_slice(&image.planes.to_le_bytes());
		group.extend_from_slice(&image.bits.to_le_bytes());
		group.extend_from_slice(&(image.data.len() as u32).to_le_bytes());
		group.extend_from_slice(&id.to_le_bytes());
	}
	resource(&mut out, RT_GROUP_ICON, GROUP_ID, GROUP_FLAGS, &group);
	out
}

/// Append a resource of type `kind` and id `id` (numbers both; `0` and `0` for the
/// empty first one) holding `data`.
fn resource(out: &mut Vec<u8>, kind: u16, id: u16, flags: u16, data: &[u8]) {
	let language = if kind == 0 { 0 } else { LANGUAGE };
	out.extend_from_slice(&(data.len() as u32).to_le_bytes());
	// The header's size: these fields, the type and the name (each 0xFFFF and a number).
	out.extend_from_slice(&32u32.to_le_bytes());
	for number in [kind, id] {
		out.extend_from_slice(&0xFFFFu16.to_le_bytes());
		out.extend_from_slice(&number.to_le_bytes());
	}
	// DataVersion, MemoryFlags, LanguageId, Version, Characteristics.
	out.extend_from_slice(&0u32.to_le_bytes());
	out.extend_from_slice(&flags.to_le_bytes());
	out.extend_from_slice(&language.to_le_bytes());
	out.extend_from_slice(&[0; 8]);
	out.extend_from_slice(data);
	out.resize(out.len().next_multiple_of(4), 0);
}
