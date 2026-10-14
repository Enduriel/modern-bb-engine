//! The `color` native class and the `createColor` / `createColorRGB` /
//! `createColorWhite` root factories.
//!
//! A `color` is one packed ARGB `u32` (alpha in the high byte). The `R`/`G`/`B`/`A`
//! properties are views over it (0–255), and `RGBA` is the packed value — so they
//! always stay consistent. Scripts create colors from hex strings at config-load
//! time (875 sites, boot-critical) and read `.R`/`.G`/`.B`.
//!
//! `varyRGB(rf, gf, bf)` randomly perturbs the color in place — see [`Color::vary`].

use squirrel::{ClassBuilder, Number, SQVM, SqStrongRef, SqTable};

use super::prng::Prng;

/// A packed ARGB color — the payload behind a `color` instance.
#[derive(Clone, Copy)]
pub struct Color {
	/// `0xAARRGGBB`: alpha in the high byte, then red, green, blue.
	pub argb: u32,
}

impl Color {
	fn channel(self, shift: u32) -> i64 {
		((self.argb >> shift) & 0xff) as i64
	}
	fn set_channel(&mut self, shift: u32, value: i64) {
		let byte = (value as u32) & 0xff;
		self.argb = (self.argb & !(0xff << shift)) | (byte << shift);
	}

	/// Randomly perturb R/G/B in place: each channel is multiplied by an
	/// independent uniform random factor in `[1 - f, 1 + f]` (its per-channel
	/// argument) and clamped to `0..=255`. Alpha is untouched.
	///
	/// The exact engine distribution is unverified — but a multiplicative
	/// `[1-f, 1+f]` jitter is what makes the one caller sensible: `hex_skill`
	/// re-rolls `varyRGB(0.75, …)` on `#ff0000` in a loop *until* `R+G+B > 150`,
	/// which only terminates (and is only a meaningful loop) if the red channel can
	/// fall well below its starting 255 — i.e. the factor must reach down to `1-f`.
	/// Uses the shared [`Prng`] so a seeded run is reproducible.
	fn vary(&mut self, prng: &Prng, rf: f64, gf: f64, bf: f64) {
		for (shift, factor) in [(16, rf), (8, gf), (0, bf)] {
			let scaled = self.channel(shift) as f64 * prng.rand_range_f(1.0 - factor, 1.0 + factor);
			self.set_channel(shift, scaled.round().clamp(0.0, 255.0) as i64);
		}
	}
}

/// Parse a color string into packed ARGB.
///
/// `createColor(text)` as the original parses it (bound at `0x5056a0`, parser
/// disassembled at `0x521e90`, edge cases probed):
/// - with a leading `#`, the rest is read as a hex `u32` (a C++ stream, `std::hex`) and
///   only its low 24 bits are used, as RGB, with alpha always 255: `#ff804020` is
///   R `0x80`, G `0x40`, B `0x20`, A 255. A failed read (or an empty string) leaves
///   the original's return slot uninitialized, so it returns stack garbage (a stale
///   pointer when probed); `None` here, and the caller uses transparent black;
/// - without `#`, the whole string is a hex `u32` read as RRGGBBAA (vanilla's usual
///   form). That read's failure is not checked.
fn parse_color(text: &str) -> Option<u32> {
	if text.is_empty() {
		return None;
	}
	match text.strip_prefix('#') {
		Some(rest) => {
			let v = read_hex_u32(rest).ok()?;
			Some(0xff00_0000 | (v & 0x00ff_ffff))
		}
		None => {
			let v = read_hex_u32(text).unwrap_or_else(|v| v);
			let (r, g, b, a) = (v >> 24, (v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff);
			Some((a << 24) | (r << 16) | (g << 8) | b)
		}
	}
}

/// A C++ `istream >> std::hex >> unsigned` read, as MSVC's runtime does it: leading
/// whitespace and an optional `0x` are skipped and hex digits are read until the first
/// other character. `Err` carries the value left in the variable on failure, which is
/// 0 both when no digit was read and on overflow (probed: vanilla's 9-digit typos such
/// as `fffffff00` come out as transparent black).
fn read_hex_u32(text: &str) -> Result<u32, u32> {
	let t = text.trim_start();
	let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
	let digits: String = t.chars().take_while(char::is_ascii_hexdigit).collect();
	if digits.is_empty() {
		return Err(0);
	}
	u32::from_str_radix(&digits, 16).map_err(|_| 0)
}

/// The ARGB of a `color` object, read through its channels, or `None` if `value` isn't
/// one. Lets render state keep plain values instead of VM references.
pub(crate) fn color_argb(vm: &SQVM, value: &SqStrongRef) -> Option<u32> {
	let channel = |name: &str| value.get_member::<i64>(vm, name).map(|v| (v & 0xff) as u32);
	Some((channel("A")? << 24) | (channel("R")? << 16) | (channel("G")? << 8) | channel("B")?)
}

/// A new `color` object for `argb`, made with the scripts' `createColor`.
pub(crate) fn color_object(vm: &SQVM, argb: u32) -> Option<SqStrongRef> {
	let create = SqTable::root(vm).get::<SqStrongRef>("createColor")?;
	let (a, rgb) = (argb >> 24, argb & 0x00ff_ffff);
	create.call_ret::<SqStrongRef, _>(vm, (format!("{rgb:06x}{a:02x}"),)).ok().flatten()
}

/// Register the `color` class and its three factory functions, sharing `prng`
/// with `Math.rand` so `varyRGB` draws from the same RNG stream.
pub(crate) fn install(vm: &mut SQVM, prng: Prng) {
	let color = ClassBuilder::<Color>::new("color")
		.property("R", |c| c.channel(16), |c, v| c.set_channel(16, v))
		.property("G", |c| c.channel(8), |c, v| c.set_channel(8, v))
		.property("B", |c| c.channel(0), |c, v| c.set_channel(0, v))
		.property("A", |c| c.channel(24), |c, v| c.set_channel(24, v))
		.property("RGBA", |c| c.argb as i64, |c, v| c.argb = v as u32)
		// varyRGB(rf, gf, bf): in-place random jitter of R/G/B (returns nothing).
		.method(
			"varyRGB",
			move |c: &mut Color, rf: Number, gf: Number, bf: Number| {
				c.vary(&prng, rf.0, gf.0, bf.0);
			},
		)
		.build(vm);

	// createColor("#rrggbb" | "rrggbbaa") -> color (see parse_color)
	let from_hex = color.clone();
	vm.register("createColor", move |vm: &SQVM, hex: &str| {
		// The original returns uninitialized memory here (see parse_color).
		from_hex.instantiate(
			vm,
			Color {
				argb: parse_color(hex).unwrap_or(0),
			},
		)
	});

	// createColorRGB(r, g, b) -> opaque color
	let from_rgb = color.clone();
	vm.register(
		"createColorRGB",
		move |vm: &SQVM, r: Number, g: Number, b: Number| {
			// Channels are 0-1 floats, scaled in f32 and truncated into a byte, which
			// wraps (`createColorRGB(10, 20, 30)` has G 236), as the original does
			// (disassembled at `0x5056c0`).
			let pack = |v: f64| {
				let scaled = (v as f32) * 255.0;
				let n = if scaled.is_nan() || !(-2_147_483_648.0..2_147_483_648.0).contains(&scaled) {
					i32::MIN
				} else {
					scaled as i32
				};
				(n as u32) & 0xff
			};
			let argb = 0xff00_0000 | (pack(r.0) << 16) | (pack(g.0) << 8) | pack(b.0);
			from_rgb.instantiate(vm, Color { argb })
		},
	);

	// createColorWhite() -> opaque white
	vm.register("createColorWhite", move |vm: &SQVM| {
		color.instantiate(vm, Color { argb: 0xffff_ffff })
	});
}
