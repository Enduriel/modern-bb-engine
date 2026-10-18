//! Engine text the bitmap fonts can't draw, shaped and kept as glyphs: cosmic-text
//! shapes it (scripts, bidi, line breaks between CJK characters), swash rasterizes each
//! new glyph, and the glyph goes into one of our glyph pages ([`GLYPH_PAGES`]), white
//! with its coverage as alpha, so the sprite program colours, lights and scales it as it
//! does a bitmap font's glyph. The renderer uploads what [`TextShaper::take_uploads`]
//! hands it before drawing.
//!
//! Shaped texts are kept by their bitmap font, text and wrap width, and shaped again
//! once the font database's revision moves (a font registered since).

use std::collections::HashMap;

use cosmic_text::{Attrs, Buffer, CacheKey, Metrics, Shaping, SwashCache, SwashContent, Wrap};

use super::FontDb;
use crate::pages::alloc::{PageAlloc, Place};

/// A glyph page's side, in texels.
pub const GLYPH_PAGE: u32 = 1024;

/// The glyph pages' texture keys, as quads name them.
pub const GLYPH_PAGES: [&str; 16] = [
	"@glyphs/0", "@glyphs/1", "@glyphs/2", "@glyphs/3", "@glyphs/4", "@glyphs/5", "@glyphs/6", "@glyphs/7",
	"@glyphs/8", "@glyphs/9", "@glyphs/10", "@glyphs/11", "@glyphs/12", "@glyphs/13", "@glyphs/14", "@glyphs/15",
];

/// The glyph page a texture key names, if it names one.
pub fn glyph_page(key: &str) -> Option<usize> {
	GLYPH_PAGES.iter().position(|k| *k == key)
}

/// A glyph to draw: its page and texels there, and where its picture sits from the
/// text's top-left, in font pixels (y down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedGlyph {
	pub page: usize,
	/// x, y, width, height in the page.
	pub texels: [u32; 4],
	pub left: f32,
	pub top: f32,
	/// The text it stands for: a byte range of the shaped text.
	pub start: usize,
	pub end: usize,
}

/// A text shaped: its glyphs, its size in font pixels, and its lines.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShapedText {
	pub glyphs: Vec<PlacedGlyph>,
	pub width: f32,
	pub height: f32,
	/// Each line's byte range of the text, in order.
	pub lines: Vec<std::ops::Range<usize>>,
}

/// A glyph's picture to put in its page.
pub struct GlyphUpload {
	pub page: usize,
	pub x: u32,
	pub y: u32,
	pub width: u32,
	pub height: u32,
	/// RGBA, top row first.
	pub rgba: Vec<u8>,
}

/// Where a rasterized glyph went (`None`: it has no picture, a space).
#[derive(Clone, Copy)]
struct Kept {
	place: Option<Place>,
	left: i32,
	top: i32,
}

/// Shapes engine text and keeps its glyphs.
pub struct TextShaper {
	db: FontDb,
	swash: SwashCache,
	glyphs: HashMap<CacheKey, Kept>,
	alloc: PageAlloc,
	uploads: Vec<GlyphUpload>,
	shaped: HashMap<(String, String, Option<u32>), (u64, ShapedText)>,
}

impl TextShaper {
	pub fn new(db: FontDb) -> TextShaper {
		TextShaper {
			db,
			swash: SwashCache::new(),
			glyphs: HashMap::new(),
			alloc: PageAlloc::new(GLYPH_PAGE),
			uploads: Vec::new(),
			shaped: HashMap::new(),
		}
	}

	/// The font database (to register fonts in).
	pub fn db(&mut self) -> &mut FontDb {
		&mut self.db
	}

	/// `text` shaped where the game uses the bitmap font `bitmap`, at that font's size,
	/// on one line or wrapped at `wrap` font pixels.
	pub fn shape(&mut self, bitmap: &str, text: &str, wrap: Option<f32>) -> &ShapedText {
		let key = (bitmap.to_string(), text.to_string(), wrap.map(f32::to_bits));
		let revision = self.db.revision();
		if self.shaped.get(&key).is_none_or(|(r, _)| *r != revision) {
			let shaped = self.shape_now(bitmap, text, wrap);
			self.shaped.insert(key.clone(), (revision, shaped));
		}
		&self.shaped[&key].1
	}

	fn shape_now(&mut self, bitmap: &str, text: &str, wrap: Option<f32>) -> ShapedText {
		self.db.load_system_fonts();
		// No font at all (no game files, no system fonts): nothing to draw with, and
		// cosmic-text can't shape without one.
		if self.db.system().db().is_empty() {
			return ShapedText::default();
		}
		let size = FontDb::size(bitmap);
		let line_height = (size * 1.25).ceil();
		let runs = self.db.runs(bitmap, text);
		let system = self.db.system();
		let mut buffer = Buffer::new(system, Metrics::new(size, line_height));
		buffer.set_wrap(if wrap.is_some() { Wrap::WordOrGlyph } else { Wrap::None });
		buffer.set_size(wrap, None);
		let spans = runs.iter().map(|r| (&text[r.range.clone()], FontDb::attrs(r)));
		buffer.set_rich_text(spans, &Attrs::new(), Shaping::Advanced, None);
		buffer.shape_until_scroll(system, false);
		let mut found = Vec::new();
		let mut shaped = ShapedText::default();
		for run in buffer.layout_runs() {
			shaped.width = shaped.width.max(run.line_w);
			shaped.height = shaped.height.max(run.line_top + run.line_height);
			let (start, end) = run.glyphs.iter().fold((usize::MAX, 0), |(s, e), g| (s.min(g.start), e.max(g.end)));
			if start <= end {
				shaped.lines.push(start..end);
			}
			for glyph in run.glyphs {
				let physical = glyph.physical((0.0, run.line_y), 1.0);
				found.push((physical.cache_key, physical.x, physical.y, glyph.start, glyph.end));
			}
		}
		for (key, x, y, start, end) in found {
			let kept = self.keep(key);
			if let Some(place) = kept.place {
				shaped.glyphs.push(PlacedGlyph {
					page: place.page,
					texels: [place.x, place.y, place.w, place.h],
					left: (x + kept.left) as f32,
					top: (y - kept.top) as f32,
					start,
					end,
				});
			}
		}
		shaped
	}

	/// The glyph `key`'s place in the pages, rasterized and queued for upload if new.
	fn keep(&mut self, key: CacheKey) -> Kept {
		if let Some(kept) = self.glyphs.get(&key) {
			return *kept;
		}
		let image = self.swash.get_image_uncached(self.db.system(), key);
		let kept = match image.filter(|i| i.placement.width > 0 && i.placement.height > 0) {
			None => Kept { place: None, left: 0, top: 0 },
			Some(image) => {
				let (w, h) = (image.placement.width, image.placement.height);
				let rgba: Vec<u8> = match image.content {
					SwashContent::Mask => image.data.iter().flat_map(|&a| [255, 255, 255, a]).collect(),
					SwashContent::Color => image.data.clone(),
					SwashContent::SubpixelMask => image.data.chunks(4).flat_map(|c| [255, 255, 255, c[1]]).collect(),
				};
				let (place, _) = self.alloc.alloc(w, h);
				let place = Place { w, h, ..place };
				self.uploads.push(GlyphUpload { page: place.page, x: place.x, y: place.y, width: w, height: h, rgba });
				Kept { place: Some(place), left: image.placement.left, top: image.placement.top }
			}
		};
		self.glyphs.insert(key, kept);
		kept
	}

	/// The glyphs rasterized since the last call, for the renderer to put in its pages.
	pub fn take_uploads(&mut self) -> Vec<GlyphUpload> {
		std::mem::take(&mut self.uploads)
	}

	/// The glyph pages in use, for the renderer to make textures for.
	pub fn pages(&self) -> usize {
		self.alloc.pages().max().map_or(0, |p| p + 1)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::fonts::Registration;

	const HAN_FONT: &[u8] = include_bytes!("../../tests/fixtures/bb_test_han_serif.otf");

	/// A shaper with only the test font, standing in for every bitmap font's Han text.
	fn shaper() -> TextShaper {
		let mut db = FontDb::new(Vec::new(), false);
		db.register(Registration {
			path: "test.otf".into(),
			data: HAN_FONT.to_vec(),
			stands_in_for: None,
			scripts: Some(vec!["Han".into()]),
			priority: 0,
		})
		.expect("the test font");
		TextShaper::new(db)
	}

	/// The characters each glyph stands for.
	fn chars(text: &str, shaped: &ShapedText) -> Vec<String> {
		shaped.glyphs.iter().map(|g| text[g.start..g.end].to_string()).collect()
	}

	#[test]
	fn a_chinese_name_is_one_glyph_a_character_each_with_its_picture() {
		let mut shaper = shaper();
		let text = "白石镇";
		let shaped = shaper.shape("cinzel_bold_20", text, None).clone();
		assert_eq!(chars(text, &shaped), ["白", "石", "镇"]);
		assert_eq!(shaped.lines.len(), 1);
		// Left to right, each with texels in a page and a picture to upload.
		assert!(shaped.glyphs.windows(2).all(|w| w[0].left < w[1].left));
		let uploads = shaper.take_uploads();
		assert_eq!(uploads.len(), 3);
		for u in &uploads {
			assert_eq!(u.rgba.len() as u32, u.width * u.height * 4);
			assert!(u.rgba.chunks(4).any(|t| t[3] > 0), "a glyph with ink");
		}
		// Shaped again: kept, nothing new to upload.
		shaper.shape("cinzel_bold_20", text, None);
		assert!(shaper.take_uploads().is_empty());
	}

	#[test]
	fn chinese_wraps_between_characters_within_the_width() {
		let mut shaper = shaper();
		let text = "黑森林流浪者的营地在北方山谷中";
		let one_line = shaper.shape("cinzel_bold_20", text, None).width;
		let wrap = (one_line / 3.0).ceil();
		let shaped = shaper.shape("cinzel_bold_20", text, Some(wrap)).clone();
		assert!(shaped.lines.len() >= 3, "{:?}", shaped.lines);
		for line in &shaped.lines {
			assert!(text.is_char_boundary(line.start) && text.is_char_boundary(line.end));
		}
		// The lines cover the text in order, every character once.
		let joined: String = shaped.lines.iter().map(|l| &text[l.clone()]).collect();
		assert_eq!(joined, text);
		assert!(shaped.width <= wrap + 0.5, "{} > {wrap}", shaped.width);
		assert_eq!(chars(text, &shaped).len(), text.chars().count());
	}

	#[test]
	fn a_font_registered_later_reshapes_the_text() {
		let mut shaper = TextShaper::new(FontDb::new(Vec::new(), false));
		let text = "白石镇";
		let before = shaper.shape("cinzel_bold_20", text, None).glyphs.len();
		shaper.db().register(Registration {
			path: "test.otf".into(),
			data: HAN_FONT.to_vec(),
			stands_in_for: Some(vec!["cinzel_bold_20".into()]),
			scripts: None,
			priority: 0,
		})
		.unwrap();
		let after = shaper.shape("cinzel_bold_20", text, None).glyphs.len();
		assert_eq!((before, after), (0, 3), "no font at all, then the registered one");
	}

	#[test]
	fn the_systems_fonts_cover_what_the_registered_ones_lack() {
		let mut shaper = TextShaper::new(FontDb::new(Vec::new(), true));
		let text = "龍";
		let shaped = shaper.shape("cinzel_bold_20", text, None).clone();
		if shaped.glyphs.is_empty() {
			eprintln!("skipping: no system font has {text}");
			return;
		}
		assert_eq!(chars(text, &shaped), ["龍"]);
	}
}
