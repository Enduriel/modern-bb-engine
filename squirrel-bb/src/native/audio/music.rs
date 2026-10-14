//! The music (`MusicMixer`; `Music.setTrackList` `0x4ad010`, its track changes
//! `0x4ad2d0`/`0x4ad4c0`; disassembled 2026-10-14):
//! - a track list replaces the last; the playing track carries on if it's the list's
//!   only track, or with `keepPlaying`, or while it's already fading out; otherwise it
//!   fades out over the cross-fade time and then the next track fades in over the same
//!   time. The two never overlap: a change takes twice the cross-fade;
//! - the first track ever is the list's first, at once and without a fade; later ones
//!   are drawn at random, never the track just played (if another exists);
//! - a track that ends is followed at once by the next, without a fade; 3 s before
//!   its end the next is picked and loaded;
//! - with the music silent (its volume 0) nothing starts: it tries again every 3 s;
//! - an empty list changes nothing.
//!
//! `Music.isPlaying()` (`0x4ae820`) is whether the current track isn't paused (it is
//! when the music's volume is 0). The original crashes when no track has ever started
//! (it doesn't check); ours answers false (a divergence, see `docs/differences.md`).

use super::manager::{FadeDone, Hook, InstanceId, MUSIC, SoundManager, Timer};

#[derive(Clone, Debug, Default)]
pub(crate) struct Music {
	pub current: Option<InstanceId>,
	pub current_name: String,
	/// The next track, picked (and loading).
	pub next: Option<String>,
	pub tracks: Vec<String>,
	/// Milliseconds.
	pub cross_fade: u32,
	pub fading_out: bool,
	pub need_pick: bool,
}

/// How long before a track's end the next is picked.
const PRELOAD_BEFORE_MS: u32 = 3000;

/// How long a silent music waits before trying again.
const RETRY_MS: u32 = 3000;

impl SoundManager {
	/// `Music.setTrackList(tracks, crossFade[, keepPlaying])`.
	pub fn set_track_list(&mut self, tracks: Vec<String>, cross_fade: u32, keep_playing: bool) {
		let tracks: Vec<String> = tracks.into_iter().filter(|t| !t.is_empty()).collect();
		if tracks.is_empty() {
			return;
		}
		let only = (tracks.len() == 1).then(|| tracks[0].clone());
		let music = &mut self.music;
		music.tracks = tracks;
		music.cross_fade = cross_fade;
		music.need_pick = true;
		music.next = None;
		let current = music.current.and_then(|id| self.instance(id).map(|i| i.paused));
		match current {
			None | Some(true) => self.music_start_next(false),
			Some(false) => {
				if self.music.fading_out || keep_playing || only.as_deref() == Some(self.music.current_name.as_str()) {
					return;
				}
				let id = self.music.current.expect("playing");
				self.music.fading_out = true;
				self.fade_out(id, cross_fade, Some(FadeDone::MusicFadedOut));
				self.music_pick_next();
			}
		}
	}

	/// `Music.isPlaying()`.
	pub fn music_playing(&self) -> bool {
		self.music.current.and_then(|id| self.instance(id)).is_some_and(|i| !i.paused)
	}

	/// Start the next track, fading it in or not (`0x4ad2d0`).
	pub(crate) fn music_start_next(&mut self, fade_in: bool) {
		if self.audible(MUSIC) == 0.0 {
			self.schedule(self.now.wrapping_add(RETRY_MS), Timer::Music);
			return;
		}
		if self.music.need_pick || self.music.next.is_none() {
			self.music_pick_next();
		}
		if let Some(old) = self.music.current.take() {
			if let Some(i) = self.instance_mut(old) {
				i.finished = true;
			}
			self.cancel_fades(old);
			self.music.fading_out = false;
		}
		let Some(file) = self.music.next.take() else { return };
		// A track that can't be read: nothing plays (the original crashes).
		let Some(id) = self.start(&file, None, MUSIC, false) else { return };
		let length = self.out.length_ms(&file);
		if let Some(i) = self.instance_mut(id) {
			i.on_finished = Some(Hook::TrackEnded);
			i.on_position = Some((length.wrapping_sub(PRELOAD_BEFORE_MS), Hook::PreloadNext));
		}
		let audible = self.audible(MUSIC) * 1.0;
		self.set_out_volume(id, audible);
		if fade_in {
			let ms = self.music.cross_fade;
			self.fade_in(id, ms);
		}
		self.play(id);
		self.music.current = Some(id);
		self.music.current_name = file;
	}

	/// Pick the next track and start loading it (`0x4ad4c0`).
	fn music_pick_next(&mut self) {
		let tracks = &self.music.tracks;
		let n = tracks.len();
		if n == 0 {
			return;
		}
		let current = &self.music.current_name;
		let file = if self.music.current.is_none() || tracks.iter().all(|t| t == current) {
			tracks[0].clone()
		} else {
			loop {
				let index = if n > 1 { self.rng.range(0, n as i64 - 1) as usize } else { 0 };
				if self.music.tracks[index] != self.music.current_name {
					break self.music.tracks[index].clone();
				}
			}
		};
		self.out.preload(&file);
		self.music.next = Some(file);
		self.music.need_pick = false;
	}

	/// The fade out of a track ended (`0x4ad700`).
	pub(crate) fn music_faded_out(&mut self) {
		self.music.fading_out = false;
		if !self.music.tracks.is_empty() {
			self.music_start_next(true);
		}
	}

	/// A track ended (`0x4ad720`).
	pub(crate) fn music_track_ended(&mut self) {
		self.music.fading_out = false;
		if !self.music.tracks.is_empty() {
			self.music_start_next(false);
		}
	}

	/// 3 s before a track's end (`0x4ad740`).
	pub(crate) fn music_preload_next(&mut self) {
		if self.music.need_pick || self.music.next.is_none() {
			self.music_pick_next();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::super::manager::tests::manager;
	use super::*;

	fn list(names: &[&str]) -> Vec<String> {
		names.iter().map(|n| n.to_string()).collect()
	}

	#[test]
	fn the_first_track_is_the_lists_first_at_once() {
		let (mut m, log, _) = manager(60_000);
		m.update(1000);
		assert!(!m.music_playing());
		m.set_track_list(list(&["music/title.ogg", "music/b.ogg"]), 6000, false);
		assert_eq!(log.borrow().started[0].1, "music/title.ogg");
		assert!(m.music_playing());
		assert_eq!(m.instance(m.music.current.unwrap()).unwrap().vol, 1.0, "no fade");
		m.set_track_list(Vec::new(), 0, false);
		assert_eq!(m.music.tracks.len(), 2, "an empty list changes nothing");
	}

	#[test]
	fn a_change_fades_out_then_in() {
		let (mut m, log, _) = manager(60_000);
		m.update(1000);
		m.set_track_list(list(&["music/a.ogg"]), 4000, false);
		m.set_track_list(list(&["music/a.ogg"]), 4000, false);
		assert!(!m.music.fading_out, "the same lone track plays on");
		m.set_track_list(list(&["music/b.ogg"]), 4000, false);
		assert!(m.music.fading_out);
		m.update(3000);
		assert_eq!(m.instance(m.music.current.unwrap()).unwrap().vol, 0.5, "halfway out");
		m.update(5000);
		assert_eq!(log.borrow().started.last().unwrap().1, "music/b.ogg", "then the next");
		m.update(7000);
		assert_eq!(m.instance(m.music.current.unwrap()).unwrap().vol, 0.5, "halfway in");
	}

	#[test]
	fn an_ended_track_is_followed_by_another() {
		let (mut m, log, ended) = manager(60_000);
		m.update(1000);
		m.set_track_list(list(&["music/a.ogg", "music/b.ogg", "music/c.ogg"]), 4000, false);
		ended.borrow_mut().push(1);
		m.update(2000);
		let started = log.borrow().started.clone();
		assert_eq!(started.len(), 2);
		assert_ne!(started[1].1, "music/a.ogg", "never the track just played");
	}

	#[test]
	fn silent_music_waits() {
		let (mut m, log, _) = manager(60_000);
		m.channels[MUSIC] = 0.0;
		m.update(1000);
		m.set_track_list(list(&["music/a.ogg"]), 4000, false);
		assert!(log.borrow().started.is_empty());
		m.channels[MUSIC] = 0.5;
		m.update(4000);
		assert_eq!(log.borrow().started.len(), 1, "tried again after 3 s");
	}
}
