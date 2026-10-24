//! Packing on workloads recorded from real sessions (`memory_inventory`, the frames'
//! regions as they came and went): the layouts the pages take, without a GPU, under the
//! same rules as [`super::FramePages`] (where a region goes, when the pages are packed
//! again).
//!
//! A workload is lines of:
//! - `a ID W H`: frame `ID`'s region (`W` × `H` texels) comes in;
//! - `f ID`: it goes (a scene's discard, or the budget);
//! - `d`: a scene discard (the frames it took follow as `f`);
//! - `p`: the frames are about to be drawn (packing happens here, if due);
//! - `m PAGES POINT`: a point the session measured, and the most pages the frames
//!   then may take (what this packer takes: fewer is better, more is a regression).

use std::collections::HashMap;

use super::PAGE;
use super::alloc::{PageAlloc, Place, UNIT};
use super::when::PackWhen;

const WORKLOADS: [(&str, &str); 2] = [
	("vanilla", include_str!("workloads/vanilla.txt")),
	("modded", include_str!("workloads/modded.txt")),
];

/// A measured point: the pages in use and the frames' texels.
struct Point {
	name: String,
	most: usize,
	pages: usize,
	texels: u64,
}

fn replay(workload: &str) -> Vec<Point> {
	let mut alloc = PageAlloc::new(PAGE);
	let mut pack = PackWhen::default();
	let mut live: HashMap<u32, Place> = HashMap::new();
	let mut points = Vec::new();
	let lines: Vec<&str> = workload.lines().filter(|l| !l.starts_with('#') && !l.is_empty()).collect();
	for (i, line) in lines.iter().enumerate() {
		let mut words = line.split(' ');
		let word = |w: Option<&str>| -> u32 { w.and_then(|w| w.parse().ok()).unwrap_or_else(|| panic!("line {line:?}")) };
		match words.next() {
			Some("a") => {
				let id = word(words.next());
				let (w, h) = (word(words.next()), word(words.next()));
				let (place, new) = alloc.alloc(w, h);
				assert!(place.w >= w && place.h >= h && place.x % UNIT == 0 && place.y % UNIT == 0);
				if new {
					pack.page_added(alloc.pages().count());
				}
				assert!(live.insert(id, place).is_none(), "{id} came in twice");
			}
			Some("f") => {
				let place = live.remove(&word(words.next())).expect("a frame in a page");
				alloc.free(place);
			}
			Some("d") => pack.discarded(lines.get(i + 1).is_some_and(|l| l.starts_with("f "))),
			Some("p") => {
				if pack.take() {
					let mut ids: Vec<u32> = live.keys().copied().collect();
					ids.sort();
					let regions: Vec<(u32, u32)> = ids.iter().map(|id| (live[id].w, live[id].h)).collect();
					if let Some((layout, places)) = alloc.repacked(&regions) {
						alloc = layout;
						live = ids.into_iter().zip(places).collect();
					}
				}
			}
			Some("m") => {
				let most = word(words.next()) as usize;
				let name = words.collect::<Vec<_>>().join(" ");
				no_overlaps(&alloc, &live, &name);
				let texels = live.values().map(|p| u64::from(p.w) * u64::from(p.h)).sum();
				points.push(Point { name, most, pages: alloc.pages().count(), texels });
			}
			_ => panic!("line {line:?}"),
		}
	}
	points
}

fn no_overlaps(alloc: &PageAlloc, live: &HashMap<u32, Place>, at: &str) {
	let mut by_page: HashMap<usize, Vec<&Place>> = HashMap::new();
	for p in live.values() {
		let (w, h) = alloc.page_size(p.page).expect("a page in use");
		assert!(p.x + p.w <= w && p.y + p.h <= h, "{p:?} outside its page at {at}");
		by_page.entry(p.page).or_default().push(p);
	}
	for places in by_page.values() {
		for (i, a) in places.iter().enumerate() {
			for b in &places[..i] {
				let apart = a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
				assert!(apart, "{a:?} and {b:?} overlap at {at}");
			}
		}
	}
}

/// At every point of the recorded sessions the frames take no more pages than this
/// packer takes (each workload's `m` lines), and no more than one beyond what their
/// texels fill.
#[test]
fn recorded_sessions_take_few_pages() {
	for (workload, text) in WORKLOADS {
		let points = replay(text);
		assert!(points.len() > 5, "{workload}: its points");
		for p in &points {
			let page = u64::from(PAGE) * u64::from(PAGE);
			let least = p.texels.div_ceil(page) as usize;
			assert!(
				p.pages <= p.most && p.pages <= least + 1,
				"{workload}, {}: {} pages (at most {}; the texels fill {least})",
				p.name,
				p.pages,
				p.most
			);
		}
	}
}
