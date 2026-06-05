//! The `color` native class and the `createColor` / `createColorRGB` /
//! `createColorWhite` root factories.
//!
//! A `color` is one packed ARGB `u32` (alpha in the high byte). The `R`/`G`/`B`/`A`
//! properties are views over it (0–255), and `RGBA` is the packed value — so they
//! always stay consistent. Scripts create colors from hex strings at config-load
//! time (875 sites, boot-critical) and read `.R`/`.G`/`.B`.
//!
//! `varyRGB(rf, gf, bf)` randomly perturbs the color in place — see [`Color::vary`].

use squirrel::{ClassBuilder, Number, SQVM};

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
/// Accepts `#rrggbb` (alpha defaults to `0xff`) and `rrggbbaa` (alpha last). A
/// leading `#` is optional; only the first 8 hex digits are used and any extra
/// trailing characters are ignored (vanilla has a few 9-char typos the original
/// engine tolerates). Anything shorter than 6 hex digits falls back to opaque black.
fn parse_color(text: &str) -> u32 {
	let body = text.strip_prefix('#').unwrap_or(text);
	let hex: String = body.chars().take_while(char::is_ascii_hexdigit).collect();
	let byte = |range: std::ops::Range<usize>| u32::from_str_radix(&hex[range], 16).unwrap_or(0);
	if hex.len() >= 8 {
		let (r, g, b, a) = (byte(0..2), byte(2..4), byte(4..6), byte(6..8));
		(a << 24) | (r << 16) | (g << 8) | b
	} else if hex.len() >= 6 {
		let (r, g, b) = (byte(0..2), byte(2..4), byte(4..6));
		0xff00_0000 | (r << 16) | (g << 8) | b
	} else {
		0xff00_0000
	}
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

	// createColor("#rrggbb" | "rrggbbaa") -> color
	let from_hex = color.clone();
	vm.register("createColor", move |vm: &SQVM, hex: &str| {
		from_hex.instantiate(
			vm,
			Color {
				argb: parse_color(hex),
			},
		)
	});

	// createColorRGB(r, g, b) -> opaque color
	let from_rgb = color.clone();
	vm.register(
		"createColorRGB",
		move |vm: &SQVM, r: Number, g: Number, b: Number| {
			let pack = |v: f64| (v as i64 as u32) & 0xff;
			let argb = 0xff00_0000 | (pack(r.0) << 16) | (pack(g.0) << 8) | pack(b.0);
			from_rgb.instantiate(vm, Color { argb })
		},
	);

	// createColorWhite() -> opaque white
	vm.register("createColorWhite", move |vm: &SQVM| {
		color.instantiate(vm, Color { argb: 0xffff_ffff })
	});
}
