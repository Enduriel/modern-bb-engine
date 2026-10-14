//! The navigator against the original game, query for query.
//!
//! `testdata/navseed_battle.txt` was captured in the original game
//! (campaign seed `navseed`, first battle): the battle map as the search
//! reads it (levels, terrain, occupants, zones of control, discovery, hiding, TV
//! values), the units, and 198 `findPath` queries under three settings variants
//! (player-like, AI-like, and one with every cost knob set). For each found path it
//! records `getCostForPath` with ample and with scarce AP/fatigue, both
//! `getTurnsRequiredForPath` overloads, and `getCostForPath`'s `Tiles`/`End` after each
//! `popLastWaypoint`, which together pin down every step of the path.

use super::cost::{self, PathCost};
use super::pathfind::{self, Query, Step};
use super::settings::NavSettings;
use crate::native::tactical_grid::TacticalGridState;
use crate::native::tactical_grid::zone::faction_mask;

const DATA: &str = include_str!("testdata/navseed_battle.txt");
/// The same battle after the probe randomised every tile's level (0-3) and hiding flag
/// and spawned 30 boulders and brush (climbing, level limits, hiding and blockers).
/// Zones of control were laid before the levels changed, so they're read as dumped.
const RICH: &str = include_str!("testdata/rich_battle.txt");
const BATTLES: [(&str, &str); 2] = [("navseed", DATA), ("rich", RICH)];

/// `Const.DefaultMovementAPCost` / `DefaultMovementFatigueCost` (config/character.nut).
const DEFAULT_AP: [i32; 9] = [0, 2, 2, 3, 3, 4, 4, 2, 4];
const DEFAULT_FATIGUE: [i32; 9] = [0, 2, 4, 6, 6, 8, 14, 6, 12];

struct Unit {
	faction: i64,
	tile: usize,
	allied: Vec<i64>,
	ap: Vec<i32>,
	fatigue: Vec<i32>,
	/// AP per level, fatigue per level, max traversable levels.
	levels: [i32; 3],
}

fn ints<T: std::str::FromStr>(s: &str) -> Vec<T> {
	s.split(',').filter(|x| !x.is_empty()).map(|x| x.parse().ok().expect("number")).collect()
}

fn set_costs(dst: &mut [i32; 16], src: &[i32]) {
	for (d, s) in dst.iter_mut().zip(src) {
		*d = *s;
	}
}

struct Battle {
	grid: TacticalGridState,
	occupant: Vec<Option<i64>>,
	/// `Some(is_actor)` for occupied tiles.
	occupant_kind: Vec<Option<bool>>,
	units: Vec<Unit>,
}

fn parse(data: &str) -> Battle {
	let mut grid = None;
	let mut occupant = Vec::new();
	let mut occupant_kind = Vec::new();
	let mut units = Vec::new();
	for line in data.lines() {
		let mut parts = line.split(' ');
		match parts.next() {
			Some("size") => {
				let w: i64 = parts.next().unwrap().parse().unwrap();
				let h: i64 = parts.next().unwrap().parse().unwrap();
				let g = TacticalGridState::for_test(w, h);
				occupant = vec![None; g.tiles.len()];
				occupant_kind = vec![None; g.tiles.len()];
				grid = Some(g);
			}
			Some("T") => {
				let g = grid.as_mut().expect("size first");
				parts.next(); // column
				for cell in parts.filter(|c| !c.is_empty()) {
					let f: Vec<&str> = cell.split(',').collect();
					let (q, r): (i64, i64) = (f[0].parse().unwrap(), f[1].parse().unwrap());
					let id = g.idx_checked(q, r).expect("tile on the map");
					let t = &mut g.tiles[id];
					t.level = f[2].parse().unwrap();
					t.type_ = f[3].parse().unwrap();
					if f[4] != "-" {
						occupant[id] = Some(f[4][..f[4].len() - 1].parse().unwrap());
						occupant_kind[id] = Some(f[4].ends_with('a'));
					}
					t.is_discovered = f[5] == "1";
					t.is_hiding_entity = f[6] == "1";
					t.tv_total = f[7].parse().unwrap();
					t.tv_level_disadvantage = f[8].parse().unwrap();
					for z in f[9].split('+').filter(|z| !z.is_empty()) {
						let (fac, rest) = z.split_once(':').unwrap();
						let fac: i64 = fac.parse().unwrap();
						let masked = rest.ends_with('m');
						let count: u8 = rest.trim_end_matches('m').parse().unwrap();
						t.zoc.counts[fac as usize] = count;
						if masked {
							t.zoc.mask.insert(fac as i64);
						}
					}
				}
			}
			Some("U") => {
				let g = grid.as_ref().expect("size first");
				parts.next(); // index
				let faction = parts.next().unwrap().parse().unwrap();
				let qr: Vec<i64> = ints(parts.next().unwrap());
				let mut u = Unit {
					faction,
					tile: g.idx_checked(qr[0], qr[1]).unwrap(),
					allied: Vec::new(),
					ap: Vec::new(),
					fatigue: Vec::new(),
					levels: [0; 3],
				};
				for p in parts {
					let (k, v) = p.split_once('=').unwrap();
					match k {
						"allied" => u.allied = ints(v),
						"apc" => u.ap = ints(v),
						"fc" => u.fatigue = ints(v),
						"lvl" => u.levels = ints::<i32>(v).try_into().unwrap(),
						_ => {}
					}
				}
				units.push(u);
			}
			_ => {}
		}
	}
	Battle {
		grid: grid.expect("a map"),
		occupant,
		occupant_kind,
		units,
	}
}

/// The probe's three settings variants for unit `u`.
fn settings(u: &Unit, variant: u32) -> NavSettings {
	let mut s = NavSettings::default();
	match variant {
		0 => {
			set_costs(&mut s.ap_costs, &u.ap);
			set_costs(&mut s.fatigue_costs, &u.fatigue);
			s.fatigue_weight = 0.25;
			s.ap_per_level = u.levels[0];
			s.fatigue_per_level = u.levels[1];
			s.zoc_cost = 4;
			s.allied = Some(faction_mask(&u.allied));
			s.allow_zoc_passing = true;
			s.is_player = true;
		}
		1 => {
			set_costs(&mut s.ap_costs, &DEFAULT_AP);
			set_costs(&mut s.fatigue_costs, &DEFAULT_FATIGUE);
			s.fatigue_weight = 0.0;
			s.ap_per_level = u.levels[0];
			s.fatigue_per_level = u.levels[1];
			s.max_level_difference = u.levels[2];
			s.allow_zoc_passing = false;
			s.zoc_cost = 20;
			s.allied = Some(faction_mask(&u.allied));
			s.hidden_cost = -1;
		}
		_ => {
			set_costs(&mut s.ap_costs, &u.ap);
			set_costs(&mut s.fatigue_costs, &u.fatigue);
			s.fatigue_weight = 0.5;
			s.ap_per_level = 3;
			s.fatigue_per_level = 2;
			s.max_level_difference = 2;
			s.height_level_cost = 3;
			s.allow_zoc_passing = true;
			s.zoc_cost = 7;
			s.allied = Some(faction_mask(&[u.faction]));
		}
	}
	s
}

fn coords(g: &TacticalGridState, id: usize) -> String {
	format!("{},{}", g.tiles[id].q, g.tiles[id].r)
}

/// The probe's rendering of a `getCostForPath` table.
fn render(g: &TacticalGridState, c: &PathCost) -> String {
	let b = |v: bool| if v { 1 } else { 0 };
	format!(
		"{}/{}/{}/{}/{}/{}/{}/{}/{}/{}/{}/{}",
		c.tiles,
		c.ap_required,
		c.fatigue_required,
		c.ap,
		c.fatigue,
		b(c.is_complete),
		b(c.missing_ap),
		b(c.missing_fatigue),
		coords(g, c.first),
		coords(g, c.end),
		coords(g, c.last_before_end),
		coords(g, c.second_last_before_end)
	)
}

fn close(expected: &str, actual: f32) -> bool {
	let e: f64 = expected.parse().unwrap();
	(e - actual as f64).abs() <= 1e-5 * e.abs().max(1.0)
}

#[test]
fn find_path_matches_the_original() {
	for (name, data) in BATTLES {
		let b = parse(data);
		let g = &b.grid;
		let mut checked = 0;
		let mut failures = Vec::new();
		for line in data.lines().filter(|l| l.starts_with("Q ")) {
			let f: Vec<&str> = line.split(' ').collect();
			let (qid, ui) = (f[1], f[2].parse::<usize>().unwrap());
			let dest: Vec<i64> = ints(f[3]);
			let dest = g.idx_checked(dest[0], dest[1]).unwrap();
			let within: i32 = f[4].parse().unwrap();
			let variant: u32 = f[5].parse().unwrap();
			let found = f[6] == "1";
			let u = &b.units[ui];
			let s = settings(u, variant);
			let q = Query {
				grid: g,
				occupant: &b.occupant,
				settings: &s,
			};
			let path = pathfind::find_path(&q, u.tile, dest, within);
			checked += 1;
			if path.is_some() != found {
				failures.push(format!("{name} Q{qid}: found {} vs original {found}", path.is_some()));
				continue;
			}
			let Some(mut path) = path else { continue };
			let occupied = |i: usize| b.occupant[i].is_some();
			let hostile = b
				.units
				.iter()
				.find(|o| !u.allied.contains(&o.faction))
				.map_or(0, |o| o.faction);
			for field in &f[7..] {
				let (k, v) = field.split_once('=').unwrap();
				let ours = match k {
					"full" => render(g, &cost::cost_for_path(g, occupied, &path, &s, u.tile, 9999, 9999)),
					"lim" => render(g, &cost::cost_for_path(g, occupied, &path, &s, u.tile, 9, 15)),
					"t3" => {
						let t = cost::turns_required(g, &path, &s, u.tile, 9, None);
						if !close(v, t) {
							failures.push(format!("{name} Q{qid} t3: {t} vs {v}"));
						}
						continue;
					}
					"t5" => {
						let t = cost::turns_required(g, &path, &s, u.tile, 9, Some((dest, hostile)));
						if !close(v, t) {
							failures.push(format!("{name} Q{qid} t5: {t} vs {v}"));
						}
						continue;
					}
					"pops" => {
						let mut out = String::new();
						let mut p: Vec<Step> = path.clone();
						for _ in 0..60 {
							if p.is_empty() {
								break;
							}
							let c = cost::cost_for_path(g, occupied, &p, &s, u.tile, 9999, 9999);
							out += &format!("{}:{};", c.tiles, coords(g, c.end));
							p.pop();
						}
						out
					}
					_ => continue,
				};
				if ours != v {
					failures.push(format!("{name} Q{qid} {k}: ours {ours} vs original {v}"));
				}
			}
			path.clear();
		}
		assert_eq!(checked, 198, "{name}");
		assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
	}
}

#[test]
fn clip_path_to_distance_matches_the_original() {
	for (name, data) in BATTLES {
		let b = parse(data);
		let g = &b.grid;
		let line = data.lines().find(|l| l.starts_with("CLIP ")).expect("a CLIP row");
		let found = line.split(' ').nth(1) == Some("1");
		let expected = line.split(' ').nth(2).unwrap_or("");
		// The probe: unit 0, default settings with its allies, to square (30, 16), within 0.
		let u = &b.units[0];
		let mut s = NavSettings::default();
		s.allied = Some(faction_mask(&u.allied));
		let dest = g.idx(30, 16).unwrap();
		let q = Query {
			grid: g,
			occupant: &b.occupant,
			settings: &s,
		};
		let path = pathfind::find_path(&q, u.tile, dest, 0);
		assert_eq!(path.is_some(), found, "{name}");
		let Some(mut path) = path else { continue };
		pathfind::clip_to_distance(g, &mut path, u.tile, 4);
		let c = cost::cost_for_path(g, |i| b.occupant[i].is_some(), &path, &s, u.tile, 9999, 9999);
		assert_eq!(render(g, &c), expected, "{name}");
	}
}

/// Rebuilding the zones from the tiles of the units that exert one with
/// `addZoneOfControl`'s rule (neighbours within one level) must give the dumped counts
/// and masks. Whether a unit exerts one is script state (`actor.hasZoneOfControl`), so
/// it's inferred: every neighbour within a level carries the unit's faction. (In this
/// battle one of the player's three units doesn't.)
#[test]
fn zones_of_control_match_the_original() {
	let b = parse(DATA);
	let exerts = |u: &Unit| {
		let level = b.grid.tiles[u.tile].level;
		(0..6)
			.filter_map(|d| b.grid.neighbour(u.tile, d))
			.filter(|&n| (b.grid.tiles[n].level - level).abs() <= 1)
			.all(|n| b.grid.tiles[n].zoc.mask.contains(u.faction as i64))
	};
	let mut g = TacticalGridState::for_test(b.grid.width, b.grid.height);
	for (t, src) in g.tiles.iter_mut().zip(&b.grid.tiles) {
		t.level = src.level;
	}
	let mut applied = 0;
	for u in b.units.iter().filter(|u| exerts(u)) {
		g.apply_zone(u.tile, u.faction, false, true);
		applied += 1;
	}
	assert_eq!(applied, b.units.len() - 1);
	for (i, (ours, theirs)) in g.tiles.iter().zip(&b.grid.tiles).enumerate() {
		assert_eq!(ours.zoc.counts, theirs.zoc.counts, "counts at tile {i}");
		assert_eq!(ours.zoc.mask, theirs.zoc.mask, "mask at tile {i}");
	}
}

/// `TVLevelDisadvantage` depends only on levels and occupancy, which the dump holds, so
/// recomputing it must give the original's value on every tile.
#[test]
fn level_disadvantage_matches_the_original() {
	for (name, data) in BATTLES {
		let b = parse(data);
		let mut g = TacticalGridState::for_test(b.grid.width, b.grid.height);
		for (t, src) in g.tiles.iter_mut().zip(&b.grid.tiles) {
			t.level = src.level;
		}
		g.calculate_tactical_values_with(|i| b.occupant_kind[i]);
		for (i, (ours, theirs)) in g.tiles.iter().zip(&b.grid.tiles).enumerate() {
			assert_eq!(ours.tv_level_disadvantage, theirs.tv_level_disadvantage, "{name} tile {i}");
		}
	}
}

/// `Tactical.queryTilesInRange` / `queryActorsInRange` against the original
/// (`testdata/range_battle.txt`: the `navseed` battle's map in this module's format, and
/// the tiles each of 56 tile queries and 14 actor queries reported, in order).
#[test]
fn range_queries_match_the_original() {
	use crate::native::tactical_grid::query::{NO_TV_FLOOR, actor_passes, tile_passes};
	use crate::native::tactical_grid::range::tiles_in_range;
	use crate::native::tactical_grid::zone::faction_mask;
	const RANGE: &str = include_str!("testdata/range_battle.txt");
	let b = parse(RANGE);
	let g = &b.grid;
	let at = |s: &str| {
		let v: Vec<i64> = ints(s);
		g.idx_checked(v[0], v[1]).expect("on the map")
	};
	let list = |s: Option<&str>| -> Vec<usize> { s.map_or(vec![], |s| s.split(';').filter(|x| !x.is_empty()).map(at).collect()) };
	let mut checked = 0;
	for line in RANGE.lines() {
		let f: Vec<&str> = line.split(' ').collect();
		match f[0] {
			"R" => {
				let (centre, min, max) = (at(f[2]), f[3].parse().unwrap(), f[4].parse().unwrap());
				let only_empty = f[5] == "1";
				let allied = (f[6] != "-").then(|| faction_mask(&ints(f[6])));
				let tv_floor = if f[7] == "-" { NO_TV_FLOOR } else { f[7].parse().unwrap() };
				let ours: Vec<usize> = tiles_in_range(g, centre, max)
					.into_iter()
					.filter(|&id| tile_passes(g, centre, id, min, only_empty, tv_floor, allied.as_ref(), b.occupant[id].is_some()))
					.collect();
				assert_eq!(ours, list(f.get(8).copied()), "{line}");
				checked += 1;
			}
			"A" => {
				let (centre, min, max) = (at(f[2]), f[3].parse().unwrap(), f[4].parse().unwrap());
				let ours: Vec<usize> = tiles_in_range(g, centre, max)
					.into_iter()
					.filter(|&id| actor_passes(g, centre, id, min, b.occupant_kind[id]))
					.collect();
				assert_eq!(ours, list(f.get(5).copied()), "{line}");
				checked += 1;
			}
			_ => {}
		}
	}
	assert_eq!(checked, 70);
}
