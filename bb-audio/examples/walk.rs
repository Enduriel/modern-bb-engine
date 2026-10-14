//! How long the game's frames take while the party walks on the world map, with its
//! sound on the default device (kira) or silent (`--mute`): each tile change sets the
//! ambience, which may load sounds. Prints the slow frames and a summary.
//!
//! ```text
//! cargo run --release -p bb-audio --example walk -- [--mute]
//! ```
//!
//! Send the sound to a null sink to hear nothing (`PIPEWIRE_NODE=<sink>`).

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use bb_audio::KiraOut;
use bb_audio::gains::rolloff_for_width;
use bb_engine::files::GameFiles;
use bb_engine::harness::{self, STEAM_DATA_DIR};
use squirrel::{Number, SqTable};

fn tile(engine: &harness::GameEngine) -> i64 {
	engine
		.run_script("::__tile <- ::World.State.getPlayer().getTile().ID;", "tile")
		.expect("tile");
	SqTable::root(engine.vm()).get::<Number>("__tile").map_or(-1, |n| n.0 as i64)
}

fn main() -> Result<(), String> {
	let mute = std::env::args().any(|a| a == "--mute");
	let dir = Path::new(STEAM_DATA_DIR);
	let io = |e: std::io::Error| e.to_string();
	let engine = harness::open_install(dir).map_err(io)?;
	if !mute {
		let files = GameFiles::open(harness::data_archives(dir).map_err(io)?).map_err(io)?;
		let (width, _, _) = engine.video().screen();
		let out = KiraOut::device(Arc::new(files), rolloff_for_width(width as u32))?;
		engine.audio().set_out(Box::new(out));
	}
	harness::boot_menu(&engine)?;
	harness::prepare_new_campaign(&engine, "rust-engine-test")?;
	harness::start_new_campaign(&engine)?;
	harness::unpause_world(&engine)?;
	for _ in 0..120 {
		harness::run_frame(&engine, 1000.0 / 60.0)?;
	}
	engine.run_script(
		r#"
		local player = ::World.State.getPlayer(), from = player.getTile();
		local nav = ::World.getNavigator(), settings = nav.createSettings();
		settings.ActionPointCosts = ::Const.World.TerrainTypeNavCost;
		settings.RoadMult = 1.0 / ::Const.World.MovementSettings.RoadMult;
		local best = null;
		foreach (d in [[12, 0], [-12, 0], [0, 12], [0, -12], [8, 8], [-8, -8]])
		{
			if (!::World.isValidTileSquare(from.SquareCoords.X + d[0], from.SquareCoords.Y + d[1])) continue;
			local to = ::World.getTileSquare(from.SquareCoords.X + d[0], from.SquareCoords.Y + d[1]);
			local path = nav.findPath(from, to, settings, 0);
			if (!path.isEmpty() && (best == null || path.getSize() > best.getSize())) best = path;
		}
		player.setPath(best);
		"#,
		"path",
	)?;
	let mut last = tile(&engine);
	let (mut changes, mut change_ms, mut other_ms, mut others) = (0, Vec::new(), 0.0, 0);
	for frame in 0..1800 {
		let t = Instant::now();
		harness::run_frame(&engine, 1000.0 / 60.0)?;
		let ms = t.elapsed().as_secs_f64() * 1000.0;
		let now = tile(&engine);
		if now != last {
			changes += 1;
			change_ms.push(ms);
			println!("frame {frame:4}: tile {last} -> {now}: {ms:.1} ms");
			last = now;
		} else {
			other_ms += ms;
			others += 1;
			if ms > 10.0 {
				println!("frame {frame:4}: {ms:.1} ms (no tile change)");
			}
		}
	}
	change_ms.sort_by(f64::total_cmp);
	let mean = change_ms.iter().sum::<f64>() / change_ms.len().max(1) as f64;
	println!(
		"{}: {changes} tile changes, their frames mean {mean:.1} ms, max {:.1} ms; other frames mean {:.2} ms",
		if mute { "muted" } else { "kira" },
		change_ms.last().copied().unwrap_or(0.0),
		other_ms / others.max(1) as f64
	);
	Ok(())
}
