//! The binding: the calls of Steam's flat C API (`steam_api_flat.h`) the game makes,
//! resolved from the library loaded at run time.
//!
//! [`Steam::start`] sets the app id in the environment (`SteamAppId`/`SteamGameId`, which
//! Steam's client reads when the API starts; no `steam_appid.txt`), loads the library,
//! resolves every function before calling any, and starts the API with
//! `SteamAPI_InitFlat`. It never calls `SteamAPI_RestartAppIfNecessary`. Since the
//! pinned SDK, Steam loads the user's stats by itself at start (no `RequestCurrentStats`;
//! until they arrive `GetAchievement` fails, as the original's did in its first moments).
//! Dropping the [`Steam`] stores the stats and shuts the API down, as the original does
//! on exit.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;

use crate::library::Library;

/// `SteamErrMsg`: the message buffer `SteamAPI_InitFlat` fills.
type ErrMsg = [c_char; 1024];

/// `ESteamAPIInitResult`'s success.
const INIT_OK: i32 = 0;

/// The functions, resolved from the library (all are `S_API ... S_CALLTYPE`, the C
/// calling convention on 64-bit systems).
#[derive(Clone, Copy)]
struct Api {
	init_flat: unsafe extern "C" fn(*mut ErrMsg) -> i32,
	shutdown: unsafe extern "C" fn(),
	run_callbacks: unsafe extern "C" fn(),
	user_stats: unsafe extern "C" fn() -> *mut c_void,
	get_achievement: unsafe extern "C" fn(*mut c_void, *const c_char, *mut bool) -> bool,
	set_achievement: unsafe extern "C" fn(*mut c_void, *const c_char) -> bool,
	get_stat_i32: unsafe extern "C" fn(*mut c_void, *const c_char, *mut i32) -> bool,
	set_stat_i32: unsafe extern "C" fn(*mut c_void, *const c_char, i32) -> bool,
	store_stats: unsafe extern "C" fn(*mut c_void) -> bool,
}

/// The exported names [`Api`] resolves, in its order.
pub const SYMBOLS: [&str; 9] = [
	"SteamAPI_InitFlat",
	"SteamAPI_Shutdown",
	"SteamAPI_RunCallbacks",
	"SteamAPI_SteamUserStats_v013",
	"SteamAPI_ISteamUserStats_GetAchievement",
	"SteamAPI_ISteamUserStats_SetAchievement",
	"SteamAPI_ISteamUserStats_GetStatInt32",
	"SteamAPI_ISteamUserStats_SetStatInt32",
	"SteamAPI_ISteamUserStats_StoreStats",
];

impl Api {
	/// Resolve every function, or name the first missing one.
	fn resolve(library: &Library) -> Result<Api, String> {
		let mut found = [std::ptr::null::<c_void>(); SYMBOLS.len()];
		for (slot, name) in found.iter_mut().zip(SYMBOLS) {
			*slot = library.symbol(name).ok_or(format!("the library has no {name}"))?;
		}
		// SAFETY: each address is the library's export of that name, whose C signature
		// (steam_api_flat.h of the pinned SDK) is the type it is cast to; all are non-null.
		unsafe {
			Ok(Api {
				init_flat: std::mem::transmute(found[0]),
				shutdown: std::mem::transmute(found[1]),
				run_callbacks: std::mem::transmute(found[2]),
				user_stats: std::mem::transmute(found[3]),
				get_achievement: std::mem::transmute(found[4]),
				set_achievement: std::mem::transmute(found[5]),
				get_stat_i32: std::mem::transmute(found[6]),
				set_stat_i32: std::mem::transmute(found[7]),
				store_stats: std::mem::transmute(found[8]),
			})
		}
	}
}

/// Check that the library at `path` loads and exports every function the binding
/// calls, without starting Steam (for tests and the packaging).
pub fn check_library(path: &Path) -> Result<(), String> {
	let library = Library::open(path)?;
	Api::resolve(&library).map(|_| ())
}

/// Steam's API, started. Not `Send` (raw pointers): used on the thread that started it.
pub struct Steam {
	api: Api,
	user_stats: *mut c_void,
	/// Declared last: unloaded after the shutdown in `drop`.
	_library: Library,
}

impl Steam {
	/// Set the app id in the environment, load the library at `path` and start Steam's
	/// API. Fails (and leaves Steam alone) if the library won't load or lacks a function,
	/// or if Steam isn't running, isn't logged in or doesn't let this user play the game.
	///
	/// # Safety
	/// Call before any other thread may read or write the environment: it sets
	/// `SteamAppId` and `SteamGameId` (`std::env::set_var`).
	pub unsafe fn start(path: &Path, app_id: u32) -> Result<Steam, String> {
		let library = Library::open(path)?;
		let api = Api::resolve(&library)?;
		// SAFETY: the caller guarantees no other thread touches the environment.
		unsafe {
			std::env::set_var("SteamAppId", app_id.to_string());
			std::env::set_var("SteamGameId", app_id.to_string());
		}
		let mut message: ErrMsg = [0; 1024];
		// SAFETY: the resolved InitFlat, with a buffer of the size it fills.
		let result = unsafe { (api.init_flat)(&mut message) };
		if result != INIT_OK {
			// SAFETY: InitFlat leaves a NUL-terminated message in the zeroed buffer.
			let text = unsafe { CStr::from_ptr(message.as_ptr()) }.to_string_lossy().into_owned();
			return Err(format!("SteamAPI_InitFlat failed ({result}): {text}"));
		}
		// SAFETY: the API started.
		let user_stats = unsafe { (api.user_stats)() };
		if user_stats.is_null() {
			// SAFETY: started above, shut down once.
			unsafe { (api.shutdown)() };
			return Err("Steam gave no ISteamUserStats".into());
		}
		Ok(Steam { api, user_stats, _library: library })
	}

	/// Dispatch Steam's pending callbacks (once a frame).
	pub fn run_callbacks(&self) {
		// SAFETY: the API is running.
		unsafe { (self.api.run_callbacks)() }
	}

	/// `GetAchievement`: `None` when the call failed.
	pub fn achievement(&self, id: &str) -> Option<bool> {
		let id = CString::new(id).ok()?;
		let mut achieved = false;
		// SAFETY: the interface pointer of the running API, a C string and an out flag.
		let ok = unsafe { (self.api.get_achievement)(self.user_stats, id.as_ptr(), &mut achieved) };
		ok.then_some(achieved)
	}

	/// `SetAchievement`.
	pub fn set_achievement(&self, id: &str) -> bool {
		let Ok(id) = CString::new(id) else { return false };
		// SAFETY: as above.
		unsafe { (self.api.set_achievement)(self.user_stats, id.as_ptr()) }
	}

	/// `GetStat` (32-bit integer): `None` when the call failed.
	pub fn stat(&self, name: &str) -> Option<i32> {
		let name = CString::new(name).ok()?;
		let mut value = 0;
		// SAFETY: as above.
		let ok = unsafe { (self.api.get_stat_i32)(self.user_stats, name.as_ptr(), &mut value) };
		ok.then_some(value)
	}

	/// `SetStat` (32-bit integer).
	pub fn set_stat(&self, name: &str, value: i32) -> bool {
		let Ok(name) = CString::new(name) else { return false };
		// SAFETY: as above.
		unsafe { (self.api.set_stat_i32)(self.user_stats, name.as_ptr(), value) }
	}

	/// `StoreStats`.
	pub fn store(&self) -> bool {
		// SAFETY: as above.
		unsafe { (self.api.store_stats)(self.user_stats) }
	}
}

impl Drop for Steam {
	/// As the original exits: store the stats, then shut the API down.
	fn drop(&mut self) {
		self.store();
		// SAFETY: started in `start`, shut down once.
		unsafe { (self.api.shutdown)() };
	}
}
