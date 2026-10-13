//! Battle visibility against the original game.
//!
//! Captured from the original game (campaign seed `navseed`, first battle,
//! 2026-10-13): the map (levels, occupants and whether they block sight,
//! hiding/bad/defensible terrain, the four TV values), the units, the tiles each of 36
//! `updateVisibility` calls saw (`IsVisibleForEntity` after clearing; 6 units × vision
//! 2/4/7 × player and an AI faction), a two-viewer player sequence, and 120
//! `hasLineOfSightTo` answers. `navseed_vision.txt` is the battle as generated (flat
//! desert); `rich_vision.txt` is the same battle after the probe randomised every tile's
//! level (0-3) and hiding flag and spawned 30 boulders and brush, so heights, hiding
//! terrain and blockers all come into play.

use std::collections::BTreeSet;

use super::TacticalGridState;
use super::visibility::Occupant;

const BATTLES: [(&str, &str); 2] = [
	("navseed", include_str!("testdata/navseed_vision.txt")),
	("rich", include_str!("testdata/rich_vision.txt")),
];

struct Battle {
	grid: TacticalGridState,
	occupants: Vec<Option<Occupant>>,
	units: Vec<usize>,
	/// Per tile: TVHeight, TVTerrain, TVLevelDisadvantage, TVTotal as dumped.
	tv: Vec<[i64; 4]>,
}

fn coords(g: &TacticalGridState, s: &str) -> usize {
	let (q, r) = s.split_once(',').unwrap();
	g.idx_checked(q.parse().unwrap(), r.parse().unwrap()).expect("tile on the map")
}

fn parse(data: &str) -> Battle {
	let mut grid: Option<TacticalGridState> = None;
	let mut occupants = Vec::new();
	let mut tv = Vec::new();
	let mut units = Vec::new();
	for line in data.lines() {
		let mut parts = line.split(' ');
		match parts.next() {
			Some("size") => {
				let w = parts.next().unwrap().parse().unwrap();
				let h = parts.next().unwrap().parse().unwrap();
				let g = TacticalGridState::for_test(w, h);
				occupants = vec![None; g.tiles.len()];
				tv = vec![[0; 4]; g.tiles.len()];
				grid = Some(g);
			}
			Some("T") => {
				let g = grid.as_mut().unwrap();
				parts.next();
				for cell in parts.filter(|c| !c.is_empty()) {
					let f: Vec<&str> = cell.split(',').collect();
					let id = g.idx_checked(f[0].parse().unwrap(), f[1].parse().unwrap()).unwrap();
					let t = &mut g.tiles[id];
					t.level = f[2].parse().unwrap();
					t.type_ = f[3].parse().unwrap();
					if f[4] != "-" {
						occupants[id] = Some(Occupant {
							identity: id as i64,
							is_actor: f[4].contains('a'),
							blocks_sight: f[4].contains('b'),
							discovered: false,
						});
					}
					t.is_hiding_entity = f[5] == "1";
					t.is_bad_terrain = f[6] == "1";
					t.is_defensible_terrain = f[7] == "1";
					tv[id] = [8, 9, 10, 11].map(|k| f[k].parse().unwrap());
				}
			}
			Some("U") => {
				let g = grid.as_ref().unwrap();
				parts.next();
				parts.next();
				units.push(coords(g, parts.next().unwrap()));
			}
			_ => {}
		}
	}
	Battle {
		grid: grid.unwrap(),
		occupants,
		units,
		tv,
	}
}

fn set(g: &TacticalGridState, list: &str) -> BTreeSet<usize> {
	list.split(';').filter(|s| !s.is_empty()).map(|s| coords(g, s)).collect()
}

fn seen_now(g: &TacticalGridState) -> BTreeSet<usize> {
	(0..g.tiles.len()).filter(|&i| g.tiles[i].vis_stamp == g.vis_stamp).collect()
}

#[test]
fn update_visibility_matches_the_original() {
	for (name, data) in BATTLES {
		let mut b = parse(data);
		let mut checked = 0;
		for line in data.lines().filter(|l| l.starts_with("V ")) {
			let f: Vec<&str> = line.split(' ').collect();
			let unit: usize = f[2].parse().unwrap();
			let vision: i64 = f[3].parse().unwrap();
			let faction: i64 = f[4].parse().unwrap();
			let expected = set(&b.grid, f.get(5).copied().unwrap_or(""));
			b.grid.clear_visibility();
			let from = b.units[unit];
			let mut occ = b.occupants.clone();
			b.grid.update_visibility(&mut occ, from, vision, faction);
			let ours = seen_now(&b.grid);
			assert_eq!(ours, expected, "{name} query {}: unit {unit}, vision {vision}, faction {faction}", f[1]);
			checked += 1;
		}
		assert_eq!(checked, 36, "{name}");
	}
}

#[test]
fn successive_player_views_match_the_original() {
	for (name, data) in BATTLES {
		let mut b = parse(data);
		let line = data.lines().find(|l| l.starts_with("S2 ")).unwrap();
		let (seen, player) = line[3..].split_once(" P ").unwrap();
		b.grid.clear_visibility();
		let mut occ = b.occupants.clone();
		b.grid.update_visibility(&mut occ, b.units[0], 7, 1);
		b.grid.update_visibility(&mut occ, b.units[1], 7, 1);
		assert_eq!(seen_now(&b.grid), set(&b.grid, seen), "{name}");
		let ours: BTreeSet<usize> = (0..b.grid.tiles.len()).filter(|&i| b.grid.tiles[i].vis_mask & 2 != 0).collect();
		assert_eq!(ours, set(&b.grid, player), "{name}");
	}
}

#[test]
fn line_of_sight_matches_the_original() {
	for (name, data) in BATTLES {
		let mut b = parse(data);
		let line = data.lines().find(|l| l.starts_with("LOS ")).unwrap();
		let mut checked = 0;
		for item in line[4..].split(' ').filter(|s| !s.is_empty()) {
			let (lhs, ans) = item.split_once('=').unwrap();
			let f: Vec<&str> = lhs.split(':').collect();
			let from = b.units[f[0].parse::<usize>().unwrap()];
			let (q, r) = f[1].split_once(',').unwrap();
			let range: i64 = f[2].parse().unwrap();
			// Off-map targets can't be expressed as a tile here; the original answers false.
			let Some(to) = b.grid.idx_checked(q.parse().unwrap(), r.parse().unwrap()) else {
				assert_eq!(ans, "0");
				continue;
			};
			let ours = b.grid.line_of_sight(&b.occupants, from, to, range);
			assert_eq!(ours, ans == "1", "{name} {item}");
			checked += 1;
		}
		assert!(checked > 50, "{name}");
	}
}

#[test]
fn tactical_values_match_the_original() {
	for (name, data) in BATTLES {
		let mut b = parse(data);
		let kinds: Vec<Option<bool>> = b.occupants.iter().map(|o| o.map(|o| o.is_actor)).collect();
		b.grid.calculate_tactical_values_with(|i| kinds[i]);
		for (i, t) in b.grid.tiles.iter().enumerate() {
			assert_eq!([t.tv_height, t.tv_terrain, t.tv_level_disadvantage, t.tv_total], b.tv[i], "{name} tile {i}");
		}
	}
}
