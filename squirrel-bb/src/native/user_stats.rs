//! Achievements and persistent stats: the natives `hasAchievement`, `updateAchievement`,
//! `getPersistentStat`, `setPersistentStat` and `storeStatistics`, over a [`UserStats`]
//! service the host chooses.
//!
//! The original calls Steam's `ISteamUserStats` straight from each native (read from
//! its binary: `hasAchievement` 0x506670, `updateAchievement` 0x506440,
//! `getPersistentStat` 0x506600, `setPersistentStat` 0x506640, `storeStatistics`
//! 0x5066a0). The natives here do the same calls in the same order, on whatever service
//! the host set: Steam's (the windowed game on a Steam install, in `bb-app`), or by
//! default [`OfflineStats`], which never unlocks anything and keeps stats in memory.
//! squirrel-bb itself never touches Steam.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use squirrel::SQVM;

/// The suffix of an achievement's progress stat: `updateAchievement("NeverGiveUp", 1, 10)`
/// counts in the stat `NeverGiveUp_s` (the original's string at 0x628a10; Steam's schema
/// for app 365360 has that stat as the achievement's progress).
pub const PROGRESS_SUFFIX: &str = "_s";

/// The user's achievements and stats, as Steam's `ISteamUserStats` offers them: the five
/// calls the original's natives make. Each answers as Steam's does: `false` (or `None`)
/// when the call failed (no such achievement or stat, stats not loaded yet, no Steam).
pub trait UserStats {
	/// `GetAchievement`: whether the achievement is unlocked, `None` if the call failed.
	fn achievement(&self, id: &str) -> Option<bool>;
	/// `SetAchievement`: unlock it (sent with the next [`store`](Self::store)).
	fn set_achievement(&self, id: &str) -> bool;
	/// `GetStat` (32-bit integer), `None` if the call failed.
	fn stat(&self, name: &str) -> Option<i32>;
	/// `SetStat` (32-bit integer).
	fn set_stat(&self, name: &str, value: i32) -> bool;
	/// `StoreStats`: send the changes to Steam.
	fn store(&self) -> bool;
}

/// The service without a platform behind it (no Steam, or a non-Steam install): no
/// achievement is ever unlocked, and stats live in memory for the run (the engine's
/// behavior before Steam support, so the scripts take the same paths).
#[derive(Default)]
pub struct OfflineStats {
	stats: RefCell<HashMap<String, i32>>,
}

impl UserStats for OfflineStats {
	fn achievement(&self, _id: &str) -> Option<bool> {
		None
	}
	fn set_achievement(&self, _id: &str) -> bool {
		false
	}
	fn stat(&self, name: &str) -> Option<i32> {
		self.stats.borrow().get(name).copied()
	}
	fn set_stat(&self, name: &str, value: i32) -> bool {
		self.stats.borrow_mut().insert(name.to_string(), value);
		true
	}
	fn store(&self) -> bool {
		false
	}
}

/// The service the natives call, which the host can swap ([`UserStatsHandle::set`]).
/// Holds no VM handles, so the engine has nothing to clear before the VM closes.
#[derive(Clone)]
pub struct UserStatsHandle(Rc<RefCell<Rc<dyn UserStats>>>);

impl Default for UserStatsHandle {
	fn default() -> Self {
		UserStatsHandle(Rc::new(RefCell::new(Rc::new(OfflineStats::default()))))
	}
}

impl UserStatsHandle {
	/// Use `service` from now on (before the scripts boot, so every call reaches it).
	pub fn set(&self, service: Rc<dyn UserStats>) {
		*self.0.borrow_mut() = service;
	}

	/// The service in use.
	pub fn get(&self) -> Rc<dyn UserStats> {
		self.0.borrow().clone()
	}
}

/// `hasAchievement(id)`: `GetAchievement`'s answer, `false` when the call failed (the
/// original reads a flag it zeroed first and ignores the call's result).
pub fn has_achievement(stats: &dyn UserStats, id: &str) -> bool {
	stats.achievement(id).unwrap_or(false)
}

/// `updateAchievement(id, value, max)`, as the original's (0x506440):
/// - an unlocked achievement is left alone (no call follows, not even `StoreStats`);
/// - with `max` not 0 and `value < max`, it counts in the progress stat `id + "_s"`: if
///   that stat reads, the achievement unlocks when the count reaches `max`, and the stat
///   is set to the new count either way;
/// - otherwise (`value >= max`, `max` 0, or the stat didn't read) it unlocks at once;
/// - then `StoreStats`.
///
/// The original's arguments are 32-bit integers, and so is its sum; it never calls
/// `IndicateAchievementProgress`.
pub fn update_achievement(stats: &dyn UserStats, id: &str, value: i32, max: i32) {
	if has_achievement(stats, id) {
		return;
	}
	let progress_stat = || format!("{id}{PROGRESS_SUFFIX}");
	let progress = if max != 0 && value < max { stats.stat(&progress_stat()) } else { None };
	match progress {
		Some(progress) => {
			let count = progress.wrapping_add(value);
			if count >= max {
				stats.set_achievement(id);
			}
			stats.set_stat(&progress_stat(), count);
		}
		None => {
			stats.set_achievement(id);
		}
	}
	stats.store();
}

/// `getPersistentStat(name)`: the stat, 0 when it didn't read (0x506600).
pub fn persistent_stat(stats: &dyn UserStats, name: &str) -> i32 {
	stats.stat(name).unwrap_or(0)
}

/// `setPersistentStat(name, value)`: set the stat, then `StoreStats` (0x506640).
pub fn set_persistent_stat(stats: &dyn UserStats, name: &str, value: i32) {
	stats.set_stat(name, value);
	stats.store();
}

/// Register the five natives into the root, calling `handle`'s service.
pub(crate) fn install(vm: &mut SQVM, handle: &UserStatsHandle) -> Vec<(&'static str, squirrel::SqStrongRef)> {
	let h = handle.clone();
	let has = vm.make_closure(move |id: &str| has_achievement(&*h.get(), id));
	let h = handle.clone();
	// Squirrel integers are 64-bit, the original's arguments 32-bit: truncated as its
	// `sq_getinteger` into an int is.
	let update = vm.make_closure(move |id: &str, value: i64, max: i64| update_achievement(&*h.get(), id, value as i32, max as i32));
	let h = handle.clone();
	let get = vm.make_closure(move |name: &str| -> i64 { i64::from(persistent_stat(&*h.get(), name)) });
	let h = handle.clone();
	let set = vm.make_closure(move |name: &str, value: i64| set_persistent_stat(&*h.get(), name, value as i32));
	let h = handle.clone();
	let store = vm.make_closure(move || {
		h.get().store();
	});
	vec![
		("hasAchievement", has),
		("updateAchievement", update),
		("getPersistentStat", get),
		("setPersistentStat", set),
		("storeStatistics", store),
	]
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A fake Steam: records every call in order, answers from its tables.
	#[derive(Default)]
	struct Fake {
		unlocked: RefCell<HashMap<String, bool>>,
		stats: RefCell<HashMap<String, i32>>,
		calls: RefCell<Vec<String>>,
	}

	impl Fake {
		fn with(achievements: &[(&str, bool)], stats: &[(&str, i32)]) -> Fake {
			let fake = Fake::default();
			for (id, on) in achievements {
				fake.unlocked.borrow_mut().insert(id.to_string(), *on);
			}
			for (name, v) in stats {
				fake.stats.borrow_mut().insert(name.to_string(), *v);
			}
			fake
		}
		fn calls(&self) -> Vec<String> {
			self.calls.borrow().clone()
		}
	}

	impl UserStats for Fake {
		fn achievement(&self, id: &str) -> Option<bool> {
			self.calls.borrow_mut().push(format!("GetAchievement {id}"));
			self.unlocked.borrow().get(id).copied()
		}
		fn set_achievement(&self, id: &str) -> bool {
			self.calls.borrow_mut().push(format!("SetAchievement {id}"));
			self.unlocked.borrow_mut().insert(id.to_string(), true);
			true
		}
		fn stat(&self, name: &str) -> Option<i32> {
			self.calls.borrow_mut().push(format!("GetStat {name}"));
			self.stats.borrow().get(name).copied()
		}
		fn set_stat(&self, name: &str, value: i32) -> bool {
			self.calls.borrow_mut().push(format!("SetStat {name} {value}"));
			self.stats.borrow_mut().insert(name.to_string(), value);
			true
		}
		fn store(&self) -> bool {
			self.calls.borrow_mut().push("StoreStats".to_string());
			true
		}
	}

	#[test]
	fn a_one_step_achievement_unlocks_at_once() {
		let fake = Fake::with(&[("Kingmaker", false)], &[]);
		update_achievement(&fake, "Kingmaker", 1, 1);
		assert_eq!(fake.calls(), ["GetAchievement Kingmaker", "SetAchievement Kingmaker", "StoreStats"]);
	}

	#[test]
	fn an_unlocked_achievement_is_left_alone() {
		let fake = Fake::with(&[("Kingmaker", true)], &[]);
		update_achievement(&fake, "Kingmaker", 1, 1);
		assert_eq!(fake.calls(), ["GetAchievement Kingmaker"]);
	}

	#[test]
	fn progress_counts_in_the_stat_until_max() {
		let fake = Fake::with(&[("NeverGiveUp", false)], &[("NeverGiveUp_s", 8)]);
		update_achievement(&fake, "NeverGiveUp", 1, 10);
		assert_eq!(
			fake.calls(),
			["GetAchievement NeverGiveUp", "GetStat NeverGiveUp_s", "SetStat NeverGiveUp_s 9", "StoreStats"]
		);
		fake.calls.borrow_mut().clear();
		update_achievement(&fake, "NeverGiveUp", 1, 10);
		assert_eq!(
			fake.calls(),
			[
				"GetAchievement NeverGiveUp",
				"GetStat NeverGiveUp_s",
				"SetAchievement NeverGiveUp",
				"SetStat NeverGiveUp_s 10",
				"StoreStats"
			]
		);
	}

	/// The original's quirk: a progress stat that doesn't read unlocks the achievement.
	#[test]
	fn a_progress_stat_that_fails_to_read_unlocks() {
		let fake = Fake::with(&[("NeverGiveUp", false)], &[]);
		update_achievement(&fake, "NeverGiveUp", 1, 10);
		assert_eq!(
			fake.calls(),
			["GetAchievement NeverGiveUp", "GetStat NeverGiveUp_s", "SetAchievement NeverGiveUp", "StoreStats"]
		);
	}

	#[test]
	fn max_zero_or_value_at_max_skips_the_stat() {
		let fake = Fake::with(&[], &[("A_s", 3)]);
		update_achievement(&fake, "A", 5, 0);
		update_achievement(&fake, "B", 10, 10);
		assert_eq!(
			fake.calls(),
			["GetAchievement A", "SetAchievement A", "StoreStats", "GetAchievement B", "SetAchievement B", "StoreStats"]
		);
	}

	#[test]
	fn persistent_stats_read_zero_on_failure_and_store_on_set() {
		let fake = Fake::with(&[], &[("CrisesDefeatedOnIronman", 5)]);
		assert_eq!(persistent_stat(&fake, "CrisesDefeatedOnIronman"), 5);
		assert_eq!(persistent_stat(&fake, "Missing"), 0);
		set_persistent_stat(&fake, "CrisesDefeatedOnIronman", 7);
		assert_eq!(
			fake.calls(),
			[
				"GetStat CrisesDefeatedOnIronman",
				"GetStat Missing",
				"SetStat CrisesDefeatedOnIronman 7",
				"StoreStats"
			]
		);
	}

	#[test]
	fn offline_never_unlocks_and_keeps_stats_for_the_run() {
		let offline = OfflineStats::default();
		update_achievement(&offline, "Kingmaker", 1, 1);
		assert!(!has_achievement(&offline, "Kingmaker"));
		assert_eq!(persistent_stat(&offline, "CrisesDefeatedOnIronman"), 0);
		set_persistent_stat(&offline, "CrisesDefeatedOnIronman", 3);
		assert_eq!(persistent_stat(&offline, "CrisesDefeatedOnIronman"), 3);
	}
}
