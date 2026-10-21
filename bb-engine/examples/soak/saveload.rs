//! Every few days: save the campaign (`bb_probe_soak_<session>_<k>`), load it back
//! (in turn from the world, as its menu's Load does, and from the main menu after
//! quitting), and compare what must survive. The save before the last is deleted once
//! its successor has loaded.

use crate::record::{Json, clip};
use crate::session::{End, Session};
use crate::world::{AT_MENU, IN_WORLD};

const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";

impl Session<'_> {
	pub fn save_and_load(&mut self) -> Result<(), End> {
		// As a player saves and loads: from the menu, which needs the map without a screen
		// over it, and pauses the world. Not now if a screen is up (the next tick tries).
		if !self.holds("quiet", "::__soakQuiet()") {
			return self.frames(10);
		}
		self.next_save_day = self.day_now + self.o.save_every_days;
		self.save_count += 1;
		let k = self.save_count;
		let name = format!("bb_probe_soak_{}_{k}", self.n);
		self.context("save");
		// Paused, and the autoplay held, until the loaded campaign is compared.
		let _ = self.script("pause", "::__soakHold = true; ::World.State.setPause(true);");
		let result = self.save_and_load_paused(k, &name);
		let _ = self.script("release", "::__soakHold = false;");
		result
	}

	fn save_and_load_paused(&mut self, k: u32, name: &str) -> Result<(), End> {
		let name = name.to_string();
		let before = self.eval("invariants", "::__soakInvariants()");
		let saved = self.script("save", &format!("::World.State.saveCampaign(\"{name}\");"));
		let file = self.save_dir.join(format!("{name}.sav"));
		let size = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
		if saved.is_err() || size == 0 {
			self.stats.save_failures += 1;
			let why = saved.err().unwrap_or_else(|| format!("no file {}", file.display()));
			self.action(&format!("SAVE FAILED {name}: {why}"));
			self.shared.error("save_failure", &crate::record::signature(&why), Json::new("save_failure").str("save", &name).str("message", &why));
			return Ok(());
		}
		self.stats.saves += 1;
		self.frames(5)?;
		let via_menu = k % 2 == 0;
		self.context(if via_menu { "load from the main menu" } else { "load from the world" });
		self.action(&format!("saved {name} ({size} bytes); loading it {}", if via_menu { "from the main menu" } else { "in the world" }));
		let loaded = if via_menu { self.load_from_menu(&name)? } else { self.load_in_world(&name)? };
		if !loaded {
			self.stats.load_failures += 1;
			let state = self.eval("state", "::__soakState(true)");
			self.action(&format!("LOAD FAILED {name}: {state}"));
			self.shared.error(
				"load_failure",
				&format!("load {}", if via_menu { "from the menu" } else { "in the world" }),
				Json::new("load_failure").str("save", &name).str("state", &state),
			);
			// What's left can't be trusted to be the campaign: a fresh one.
			return Err(End::Stalled(format!("loading {name} failed: {}", clip(&state, 300))));
		}
		self.stats.loads += 1;
		let after = self.eval("invariants", "::__soakInvariants()");
		if before != after {
			self.stats.invariant_mismatches += 1;
			let first = first_difference(&before, &after);
			self.action(&format!("MISMATCH after loading {name}: {first}"));
			self.shared.error(
				"invariant_mismatch",
				&crate::record::signature(&first),
				Json::new("invariant_mismatch").str("save", &name).str("first_difference", &first).str("before", &clip(&before, 8000)).str("after", &clip(&after, 8000)),
			);
		} else {
			self.action(&format!("loaded {name}: as saved"));
		}
		Json::new("save_load").str("save", &name).int("bytes", size as i64).int("via_menu", via_menu as i64).int("same", (before == after) as i64).int("day", self.day_now).emit();
		if k > 1 {
			let _ = std::fs::remove_file(self.save_dir.join(format!("bb_probe_soak_{}_{}.sav", self.n, k - 1)));
		}
		let _ = self.script("after load", "::__autoplayEngage = true; ::__autoplayFight = true; ::__autoplayDieAsPlayers = true; ::World.State.m.IsAutosaving = false; ::World.State.setAutoPause(false); ::World.State.setPause(false);");
		Ok(())
	}

	/// Load as the world menu's Load Campaign does.
	fn load_in_world(&mut self, name: &str) -> Result<bool, End> {
		let _ = self.script("load", &format!("::World.State.campaign_menu_module_onLoadPressed(\"{name}\");"));
		self.frames(2)?;
		self.until(6000, IN_WORLD)
	}

	/// Quit to the main menu, and load from its Load Campaign page.
	fn load_from_menu(&mut self, name: &str) -> Result<bool, End> {
		// The menu closes before quitting, as the window's quit does.
		let _ = self.script("quit", "::World.State.m.MenuStack.popAll(true); ::World.State.exitGame();");
		if !self.until(6000, AT_MENU)? {
			return Ok(false);
		}
		let _ = self.script("load menu", &format!("{MENU}.getMainMenuModule().onLoadCampaignButtonPressed();"));
		self.frames(2)?;
		let _ = self.script("load", &format!("{MENU}.getLoadCampaignMenuModule().onLoadButtonPressed(\"{name}\");"));
		self.until(6000, IN_WORLD)
	}
}

/// Where two invariant lines first differ, as the tokens on each side.
fn first_difference(a: &str, b: &str) -> String {
	let (ta, tb): (Vec<&str>, Vec<&str>) = (a.split(' ').collect(), b.split(' ').collect());
	for i in 0..ta.len().max(tb.len()) {
		let (x, y) = (ta.get(i).copied().unwrap_or("(end)"), tb.get(i).copied().unwrap_or("(end)"));
		if x != y {
			return format!("{} -> {}", clip(x, 200), clip(y, 200));
		}
	}
	String::from("(same tokens)")
}
