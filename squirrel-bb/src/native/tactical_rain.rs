//! A battle's rain, `Tactical.getWeather()`'s `createRainSettings()` / `buildRain(s)` /
//! `IsRaining` (disassembled: `0x4df940`, `0x4e1ac0`, update `0x4e0120`).
//! Snow, a blizzard and falling leaves are rain with other brushes.
//!
//! - The settings are the weather's own (`createRainSettings` resets and returns them):
//!   drop counts, speeds, alphas and scales, a lifetime, a fall direction, the drop
//!   brushes (`clearDropBrushes`, `addDropBrush`) and the splat brush.
//! - `buildRain` replaces the rain: it chooses how many drops (between `MinDrops` and
//!   `MaxDrops`), scaled by the screen's width over 1024, and starts with three quarters
//!   of them and of the splats, all in and above what the camera shows.
//! - Each `Tactical.update()`, last: a drop falls along the direction (each one in
//!   seven a little slanted) by the frame's virtual time, fades in over 0.5 s, and after
//!   its lifetime fades out over 1.2 to 2 s and goes; new ones fill up to the count in
//!   the camera's view now, so the rain follows the camera. Splats, short marks on the
//!   ground, fade out over 500 ms and come back, each with an 85% chance a frame.
//! - `IsRaining` is whether there are drops. `weather.clear()` and
//!   `Tactical.clearScene()` stop the rain.
//!
//! All of it is drawing: its randomness is the engine's own (a stream of its own here),
//! never the scripts'. Fades run on real time.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Number, Retained, SQVM, SqStrongRef, SqTable};

use super::sprite_anim::{Anim, virtual_ms};
use super::sprite_color::{SpriteColor, real_time};
use super::{EngineRng, Sprites, TacticalGrid, VideoSettings};
use crate::scene::tactical::{SceneRainDrop, SceneRainSplat};

/// How far past the view's sides (and above its top) new drops start.
const MARGIN: f32 = 600.0;
const FADE_IN_MS: i64 = 500;
const SPLAT_FADE_MS: i64 = 500;

/// The rain settings (`weather+0x74`) and their defaults (`0x4df7f0`).
#[derive(Clone, Debug)]
struct Settings {
	min_scale: f32,
	max_scale: f32,
	min_velocity: f32,
	max_velocity: f32,
	min_alpha: f32,
	max_alpha: f32,
	splat_alpha: f32,
	min_drops: i64,
	max_drops: i64,
	num_splats: i64,
	drop_lifetime_ms: i64,
	scale_with_time: bool,
	direction: (f32, f32),
	drop_brushes: Vec<String>,
	splat_brush: String,
}

impl Default for Settings {
	fn default() -> Self {
		Settings {
			min_scale: 1.0,
			max_scale: 1.0,
			min_velocity: 1.0,
			max_velocity: 1.0,
			min_alpha: 1.0,
			max_alpha: 1.0,
			splat_alpha: 1.0,
			min_drops: 1,
			max_drops: 1,
			num_splats: 0,
			drop_lifetime_ms: 600,
			scale_with_time: false,
			direction: (-0.3, -0.7),
			drop_brushes: vec!["rain_01".into()],
			splat_brush: "rainsplat_01".into(),
		}
	}
}

struct Drop {
	brush: String,
	pos: (f32, f32),
	velocity: f32,
	slant: f32,
	scale: f32,
	born_ms: f64,
	color: SpriteColor,
	dying: bool,
}

struct Splat {
	tile: Option<usize>,
	brush: String,
	pos: (f32, f32),
	scale: f32,
	color: SpriteColor,
	/// Its brush's frames, from its first update (see `sprite_anim`).
	anim: Anim,
}

struct State {
	grid: TacticalGrid,
	video: VideoSettings,
	sprites: Sprites,
	rng: EngineRng,
	settings: Settings,
	/// The drop count `buildRain` chose (settings `+0x24`).
	chosen: i64,
	drops: Vec<Drop>,
	splats: Vec<Splat>,
	/// The weather table (its `IsRaining`), once installed.
	weather: Option<SqStrongRef>,
	/// Counts the rains built and cleared (for a scene holding the rain, see
	/// `scene::steps`).
	generation: u64,
}

/// What the camera shows and how much rain it gets.
struct View {
	sides: [f32; 4],
	/// The screen's width over 1024.
	factor: f32,
}

impl State {
	fn view(&self) -> View {
		let (w, h, _) = self.video.screen();
		View {
			sides: self.grid.view((w, h)),
			factor: w as f32 / 1024.0,
		}
	}

	fn new_drop(&mut self, v: &View, now_ms: f64, now: f32) {
		let [left, right, bottom, top] = v.sides;
		let s = &self.settings;
		let pos = (self.rng.between(left - MARGIN, right + MARGIN), self.rng.between(bottom, top + MARGIN));
		let alpha = (self.rng.between(s.min_alpha, s.max_alpha) * 255.0) as i64;
		let brush = match s.drop_brushes.len() {
			0 => String::new(),
			n => s.drop_brushes[self.rng.range(0, n as i64 - 1) as usize].clone(),
		};
		let velocity = self.rng.between(s.min_velocity, s.max_velocity);
		let slant = if self.rng.range(1, 100) <= 15 { -0.06 } else { 0.0 };
		let scale = self.rng.between(s.min_scale, s.max_scale);
		let mut color = SpriteColor::new(0x00ff_ffff);
		color.fade_to_alpha(alpha, FADE_IN_MS, now);
		self.drops.push(Drop {
			brush,
			pos,
			velocity,
			slant,
			scale,
			born_ms: now_ms,
			color,
			dying: false,
		});
	}

	fn new_splat(&mut self, v: &View, now: f32) {
		let [left, right, bottom, top] = v.sides;
		let pos = (self.rng.between(left, right), self.rng.between(bottom, top));
		let tile = self.grid.tile_at(f64::from(pos.0), f64::from(pos.1));
		let scale = 0.7 + 0.3 * self.rng.unit();
		let alpha = ((self.settings.splat_alpha * (0.65 + 0.25 * self.rng.unit())).min(1.0) * 255.0) as i64;
		let mut color = SpriteColor::new(0x00ff_ffff);
		color.set_alpha(alpha);
		color.fade_to_alpha(0, SPLAT_FADE_MS, now);
		self.splats.push(Splat {
			tile,
			brush: self.settings.splat_brush.clone(),
			pos,
			scale,
			color,
			anim: Anim::default(),
		});
	}

	fn clear(&mut self) {
		self.generation += 1;
		self.settings = Settings::default();
		self.chosen = 0;
		self.drops.clear();
		self.splats.clear();
	}

	/// One `Tactical.update()`: `delta` seconds of virtual time on, at real time `now`
	/// and virtual millisecond `now_ms`.
	fn advance(&mut self, delta: f32, now: f32, now_ms_virtual: u32) {
		if self.drops.is_empty() && self.splats.is_empty() && self.chosen == 0 {
			return;
		}
		let now_ms = f64::from(now) * 1000.0;
		let s = self.settings.clone();
		let lifetime = s.drop_lifetime_ms as f64;
		let rng = self.rng.clone();
		self.drops.retain_mut(|d| {
			let age = now_ms - d.born_ms;
			// Commit a fade that has run its course.
			d.color.draw(now);
			if !d.dying && d.color.is_fading_done() && age >= lifetime {
				let out = rng.range(1200, 2000) as f64 - (age - lifetime);
				d.color.fade_to_alpha(0, out.max(0.0) as i64, now);
				d.dying = true;
			}
			if d.dying && d.color.is_fading_done() {
				return false;
			}
			d.pos.0 += d.velocity * (s.direction.0 + d.slant) * delta;
			d.pos.1 += d.velocity * (s.direction.1 - d.slant) * delta;
			true
		});
		let v = self.view();
		let target = (self.chosen as f32 * v.factor).round() as usize;
		while self.drops.len() < target {
			self.new_drop(&v, now_ms, now);
		}
		self.splats.retain_mut(|sp| {
			sp.color.draw(now);
			!sp.color.is_fading_done()
		});
		let target = (s.num_splats as f32 * v.factor).round() as usize;
		while self.splats.len() < target {
			if self.rng.range(1, 100) > 85 {
				break;
			}
			self.new_splat(&v, now);
		}
		for sp in &mut self.splats {
			sp.anim.step_on(&self.sprites, &sp.brush, now_ms_virtual);
		}
	}

	/// Keep the weather's `IsRaining` as it is now.
	fn note_raining(&self, vm: &SQVM) {
		if let Some(w) = &self.weather {
			let _ = w.set_member(vm, "IsRaining", !self.drops.is_empty());
		}
	}
}

/// The battle's rain (cheap to clone; clones share it).
#[derive(Clone)]
pub(crate) struct TacticalRain(Rc<RefCell<State>>);

/// Read the script settings table `t` into `s`.
fn read_settings(vm: &SQVM, t: &SqStrongRef, s: &mut Settings) {
	let float = |name: &str, v: &mut f32| {
		if let Some(n) = t.get_member::<Number>(vm, name) {
			*v = n.0 as f32;
		}
	};
	float("MinScale", &mut s.min_scale);
	float("MaxScale", &mut s.max_scale);
	float("MinVelocity", &mut s.min_velocity);
	float("MaxVelocity", &mut s.max_velocity);
	float("MinAlpha", &mut s.min_alpha);
	float("MaxAlpha", &mut s.max_alpha);
	float("SplatAlpha", &mut s.splat_alpha);
	let int = |name: &str, v: &mut i64| {
		if let Some(n) = t.get_member::<Number>(vm, name) {
			*v = n.0 as i64;
		}
	};
	int("MinDrops", &mut s.min_drops);
	int("MaxDrops", &mut s.max_drops);
	int("NumSplats", &mut s.num_splats);
	int("DropLifetime", &mut s.drop_lifetime_ms);
	if let Some(b) = t.get_member::<bool>(vm, "ScaleDropsWithTime") {
		s.scale_with_time = b;
	}
	if let Some(brush) = t.get_member::<String>(vm, "SplatBrush") {
		s.splat_brush = brush;
	}
	if let Some(d) = t.get_member::<SqStrongRef>(vm, "Direction") {
		let axis = |a: &str| d.get_member::<Number>(vm, a).map(|n| n.0 as f32);
		if let (Some(x), Some(y)) = (axis("X"), axis("Y")) {
			s.direction = (x, y);
		}
	}
}

impl TacticalRain {
	pub(crate) fn new(grid: &TacticalGrid, video: VideoSettings, sprites: Sprites, rng: EngineRng) -> Self {
		TacticalRain(Rc::new(RefCell::new(State {
			grid: grid.clone(),
			video,
			sprites,
			rng,
			settings: Settings::default(),
			chosen: 0,
			drops: Vec::new(),
			splats: Vec::new(),
			weather: None,
			generation: 0,
		})))
	}

	/// Whether it rains (or splats are still fading), and the rain's generation: a new one
	/// for each rain built or cleared.
	pub(crate) fn state(&self) -> (bool, u64) {
		let s = self.0.borrow();
		(!s.drops.is_empty() || !s.splats.is_empty(), s.generation)
	}

	/// Make the battle weather's rain natives real, and run the rain at the end of
	/// `Tactical.update()` and end it in `Tactical.clearScene()`. Call after everything
	/// else on `Tactical` is installed.
	pub(crate) fn install(&self, vm: &mut SQVM) {
		fn tactical(vm: &SQVM) -> Option<SqTable<'_>> {
			SqTable::root(vm).get::<SqTable>("Tactical")
		}
		let Some(weather) = tactical(vm)
			.and_then(|t| t.get::<SqStrongRef>("getWeather"))
			.and_then(|f| f.call_ret::<SqStrongRef, _>(vm, ()).ok().flatten())
		else {
			return;
		};
		self.0.borrow_mut().weather = Some(weather.clone());
		// createRainSettings(): the weather's own settings, reset; a table with every field
		// at its default, and the drop-brush methods.
		let clear_brushes = {
			let st = self.0.clone();
			vm.make_closure(move || st.borrow_mut().settings.drop_brushes.clear())
		};
		let add_brush = {
			let st = self.0.clone();
			vm.make_closure(move |brush: String| st.borrow_mut().settings.drop_brushes.push(brush))
		};
		let create = {
			let st = self.0.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<SqStrongRef, String> {
				let d = Settings::default();
				st.borrow_mut().settings = d.clone();
				let mut t = SqTable::new(vm);
				for (k, v) in [
					("MinScale", d.min_scale),
					("MaxScale", d.max_scale),
					("MinVelocity", d.min_velocity),
					("MaxVelocity", d.max_velocity),
					("MinAlpha", d.min_alpha),
					("MaxAlpha", d.max_alpha),
					("SplatAlpha", d.splat_alpha),
				] {
					t.set(k, f64::from(v));
				}
				for (k, v) in [("MinDrops", d.min_drops), ("MaxDrops", d.max_drops), ("NumSplats", d.num_splats), ("DropLifetime", d.drop_lifetime_ms)] {
					t.set(k, v);
				}
				t.set("ScaleDropsWithTime", d.scale_with_time);
				t.set("SplatBrush", d.splat_brush.clone());
				let dir = SqTable::root(vm)
					.get::<SqStrongRef>("createVec")
					.and_then(|f| f.call_ret::<SqStrongRef, _>(vm, (f64::from(d.direction.0), f64::from(d.direction.1))).ok().flatten())
					.ok_or("createRainSettings: no createVec")?;
				t.set("Direction", dir);
				t.set("clearDropBrushes", clear_brushes.clone());
				t.set("addDropBrush", add_brush.clone());
				Ok(t.to_strong())
			})
		};
		// buildRain(settings): the rain anew.
		let build = {
			let st = self.0.clone();
			vm.make_closure(move |vm: &SQVM, settings: Retained| {
				let mut s = st.borrow_mut();
				let mut settings_now = s.settings.clone();
				read_settings(vm, &settings.0, &mut settings_now);
				s.settings = settings_now;
				s.generation += 1;
				s.drops.clear();
				s.splats.clear();
				let (min, max) = (s.settings.min_drops, s.settings.max_drops);
				s.chosen = if max > min { s.rng.range(min, max) } else { min };
				let v = s.view();
				let now = real_time(vm);
				let now_ms = f64::from(now) * 1000.0;
				for _ in 0..(s.chosen as f32 * v.factor * 0.75) as usize {
					s.new_drop(&v, now_ms, now);
				}
				for _ in 0..(s.settings.num_splats as f32 * v.factor * 0.75) as usize {
					s.new_splat(&v, now);
				}
				s.note_raining(vm);
			})
		};
		let inner_clear = weather.get_member::<SqStrongRef>(vm, "clear");
		let clear = {
			let st = self.0.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
				if let Some(inner) = &inner_clear {
					inner.call(vm, ())?;
				}
				let mut s = st.borrow_mut();
				s.clear();
				s.note_raining(vm);
				Ok(())
			})
		};
		for (name, f) in [("createRainSettings", create), ("buildRain", build), ("clear", clear)] {
			weather.set_member(vm, name, f).expect("the weather table");
		}
		let inner_update = tactical(vm).and_then(|t| t.get::<SqStrongRef>("update"));
		let update = {
			let st = self.0.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
				if let Some(inner) = &inner_update {
					inner.call(vm, ())?;
				}
				// `Time.getDelta()`: the frame's virtual seconds (none while paused).
				let delta = SqTable::root(vm)
					.get::<SqTable>("Time")
					.and_then(|t| t.get::<SqStrongRef>("getDelta"))
					.and_then(|f| f.call_ret::<Number, _>(vm, ()).ok().flatten())
					.map_or(0.0, |n| n.0 as f32);
				let mut s = st.borrow_mut();
				s.advance(delta, real_time(vm), virtual_ms(vm));
				s.note_raining(vm);
				Ok(())
			})
		};
		let inner_clear_scene = tactical(vm).and_then(|t| t.get::<SqStrongRef>("clearScene"));
		let clear_scene = {
			let st = self.0.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
				{
					let mut s = st.borrow_mut();
					s.clear();
					s.note_raining(vm);
				}
				match &inner_clear_scene {
					Some(inner) => inner.call(vm, ()),
					None => Ok(()),
				}
			})
		};
		if let Some(mut t) = tactical(vm) {
			t.set("update", update);
			t.set("clearScene", clear_scene);
		}
	}

	/// The drops and the splats as drawn at real time `now`.
	pub(crate) fn scene(&self, now: f32) -> (Vec<SceneRainDrop>, Vec<SceneRainSplat>) {
		let s = self.0.borrow();
		let lifetime = s.settings.drop_lifetime_ms as f64;
		let drops = s
			.drops
			.iter()
			.map(|d| {
				let scale = if s.settings.scale_with_time {
					let age = f64::from(now) * 1000.0 - d.born_ms;
					d.scale * (1.0 - (age / (lifetime + 3000.0)).min(1.0)).max(0.5) as f32
				} else {
					d.scale
				};
				SceneRainDrop {
					brush: d.brush.clone(),
					pos: d.pos,
					scale,
					color: d.color.shown(now),
				}
			})
			.collect();
		let splats = s
			.splats
			.iter()
			.map(|sp| SceneRainSplat {
				tile: sp.tile,
				brush: sp.brush.clone(),
				pos: sp.pos,
				scale: sp.scale,
				color: sp.color.shown(now),
				frame: sp.anim.frame(),
			})
			.collect();
		(drops, splats)
	}

	/// Release the weather table. Call before the VM is closed.
	pub(crate) fn clear(&self) {
		self.0.borrow_mut().weather = None;
	}
}
