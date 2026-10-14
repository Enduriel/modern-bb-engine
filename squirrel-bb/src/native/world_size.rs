//! The world map's size follows `Const.World.Settings` (the variable world size, a
//! deliberate divergence).
//!
//! Vanilla's size is `Const.World.Settings.SizeX/SizeY` (140 × 140; the desert DLC's
//! script sets `SizeY` to 170 as it loads). The world generator copies them into its
//! `m.MinX/MinY` once, when MapGen builds it in `root_state.onInit`, and the world state
//! makes the map from those. Scenarios and worldgen (the snowline) read the settings
//! themselves, live. The original clamps the map to 140 × 170, so a larger size crashes
//! it; ours makes any size, and:
//! - a size set before `root_state.onInit` (a mod, as it loads) is copied as vanilla
//!   copies it;
//! - a size set later (at the main menu: a mod's settings, the console) is taken when the
//!   next campaign fetches the generator (`MapGen.get`), so it needs no restart, and
//!   stays in step with what scenarios and the snowline read;
//! - the generator takes the settings only when they changed since it last took them,
//!   so a mod that sets the generator's `m.MinX/MinY` itself (as World Parameters once
//!   did, in a `startNewCampaign` hook) keeps its size.

use squirrel::SQVM;

/// Wrap `::MapGen.get` (on the MapGen instance) so that the world generator follows the
/// settings (see the module docs). Call right after `root_state.onInit` built MapGen;
/// without a MapGen (a test's bare root state) it does nothing.
pub(crate) fn follow_settings(vm: &SQVM) -> Result<(), String> {
	let install = vm.compile(FOLLOW, "world size follows the settings")?;
	install.call(vm, ())
}

const FOLLOW: &str = r#"
if (!("MapGen" in getroottable()) || !("Const" in getroottable()) || !("World" in ::Const)
	|| !("Settings" in ::Const.World))
	return;
// What the generator last took: what it copied as MapGen built it.
local taken = { X = ::Const.World.Settings.SizeX, Y = ::Const.World.Settings.SizeY };
local get = ::MapGen.get;
::MapGen.get <- function( _name )
{
	local template = get.call(this, _name);
	local settings = ::Const.World.Settings;
	if (_name == "world.worldmap_generator" && template != null
		&& (settings.SizeX != taken.X || settings.SizeY != taken.Y))
	{
		taken.X = settings.SizeX;
		taken.Y = settings.SizeY;
		template.m.MinX = taken.X;
		template.m.MinY = taken.Y;
	}
	return template;
};
"#;
