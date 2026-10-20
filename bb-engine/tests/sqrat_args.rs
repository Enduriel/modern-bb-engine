//! Native arguments convert as the original's Sqrat bindings convert them, and
//! `toHash` is the original's (both disassembled 2026-10-20):
//! - a save stream's string parameter takes any value's `tostring()` text, its bool
//!   parameter any value's truth: mod_stronghold writes an array with `writeString`
//!   (`stronghold_enemy_faction.nut:131`), and its campaigns must still save and load;
//! - `toHash` of a string is its content hash (Squirrel 3.0's, the same in every run),
//!   of an integer or bool the value, of a float the value truncated as an `f32`.
//!
//! Skips without the Steam install.

use std::path::Path;

use bb_engine::harness::{self, GameEngine};
use squirrel::SqTable;

fn eval_string(engine: &GameEngine, expr: &str) -> String {
	engine.run_script(&format!("::__v <- ({expr}).tostring();"), "eval").unwrap_or_else(|e| panic!("{expr}: {e}"));
	SqTable::root(engine.vm()).get::<String>("__v").expect(expr)
}

/// Squirrel 3.0's string hash in its 32-bit build (`_hashstr`: one character a step,
/// the step `(l >> 5) | 1`; MSVC's `char` is signed), as a 32-bit script integer.
fn hash_30(s: &[u8]) -> i64 {
	let mut l = s.len();
	let mut h = l as u32;
	let step = (l >> 5) | 1;
	let mut i = 0;
	while l >= step {
		let c = s[i] as i8 as i16 as u16 as u32;
		h ^= (h << 5).wrapping_add(h >> 2).wrapping_add(c);
		i += 1;
		l -= step;
	}
	h as i32 as i64
}

#[test]
fn to_hash_is_the_originals() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	let long = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJ";
	for s in ["abc", "", "a", "tactical.combat", long] {
		assert_eq!(eval_string(&engine, &format!("::toHash(\"{s}\")")), hash_30(s.as_bytes()).to_string(), "{s:?}");
	}
	// A string made at run time hashes as the same text written out.
	assert_eq!(eval_string(&engine, "::toHash(\"ab\" + \"c\")"), hash_30(b"abc").to_string());
	assert_eq!(eval_string(&engine, "::toHash(\"\\xe9t\")"), hash_30(&[0xe9, b't']).to_string(), "signed chars");
	for (expr, want) in [
		("5", "5"),
		("-5", "-5"),
		("true", "1"),
		("false", "0"),
		("null", "0"),
		("5.7", "5"),
		("-5.7", "-5"),
		("16777217.0", "16777216"),
		("1e10", "-2147483648"),
	] {
		assert_eq!(eval_string(&engine, &format!("::toHash({expr})")), want, "toHash({expr})");
	}
	// Anything else: its identity.
	assert_eq!(eval_string(&engine, "(function() { local t = {}; return ::toHash(t) == ::toHash(t); })()"), "true");
	assert_eq!(eval_string(&engine, "::toHash({}) == ::toHash([])"), "false");
}

#[test]
fn a_stronghold_style_save_round_trips() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	// The world's statistics (a script object that outlives a load) save and load more
	// after their own: every kind of value through every write.
	engine
		.run_script(
			r#"
			::World.State.setPause(true);
			local st = ::World.Statistics;
			local ser = st.onSerialize, de = st.onDeserialize;
			st.onSerialize <- function( _out )
			{
				ser.call(this, _out);
				_out.writeString(::Const.Music.BeastsTracks);  // as stronghold_enemy_faction.nut:131
				_out.writeString(null);
				_out.writeString(5.5);
				_out.writeString(12);
				_out.writeString(true);
				_out.writeBool(1);
				_out.writeBool(0);
				_out.writeBool("");
				_out.writeBool(0.0);
				_out.writeBool(null);
				_out.writeBool([]);
				_out.writeU8(300);
				_out.writeU8("x");
				_out.writeI32(-7.9);
				_out.writeF32(3);
				_out.writeU16(null);
				local meta = _out.getMetaData();
				meta.setString("bb_probe_array", [1]);
				meta.setString("bb_probe_int", 42);
				_out.writeString("end");
			};
			st.onDeserialize <- function( _in )
			{
				de.call(this, _in);
				local got = [];
				for (local i = 0; i < 5; i++) got.push(_in.readString());
				for (local i = 0; i < 6; i++) got.push(_in.readBool());
				got.push(_in.readU8());
				got.push(_in.readU8());
				got.push(_in.readI32());
				got.push(_in.readF32());
				got.push(_in.readU16());
				got.push(_in.getMetaData().getString("bb_probe_int"));
				got.push(_in.getMetaData().getString("bb_probe_array"));
				got.push(_in.readString());
				local text = "";
				foreach (v in got) text += v + "|";
				::__loaded <- text;
			};
			::World.State.saveCampaign("bb_probe_sqrat_args");
			::World.State.m.CampaignLoadTime = -100.0;
			::World.State.loadCampaign("bb_probe_sqrat_args");
			"#,
			"save and load",
		)
		.expect("the campaign saves and loads");
	let loaded = SqTable::root(engine.vm()).get::<String>("__loaded").expect("loaded");
	let parts: Vec<&str> = loaded.split('|').collect();
	assert!(parts[0].starts_with("(array : 0x"), "an array's text: {loaded}");
	let want = [
		"null", "5.5", "12", "true", // strings: their text
		"true", "false", "true", "false", "false", "true", // bools: their truth
		"44", "0", "-7", "3", "0", // numbers: cut to the width; a non-number is 0
		"42", // metadata: a string parameter too
	];
	assert_eq!(&parts[1..17], &want[..], "{loaded}");
	assert!(parts[17].starts_with("(array : 0x"), "{loaded}");
	assert_eq!(parts[18], "end", "the stream stays in step: {loaded}");
}
