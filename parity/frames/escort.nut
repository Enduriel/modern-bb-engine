// Escorting a caravan, as frame scenes (scenes.js walks it in the original,
// `frame_scenes --scenes` in ours): a settlement's escort_caravan_contract taken and
// accepted as the town screen's buttons do it (start, then the Overview's "I accept"
// sets the state Running), the caravan on its way with the company riding along, then
// the contract cancelled. Each scene logs the company's and the caravan's state.
//
// The original's side:
//   python3 "$BB_PROBE_RUNNER" parity/frames/escort.nut \
//       --js parity/frames/scenes.js --mode campaign --seed bbuitest \
//       --size 2560x1440 --shots OUT/original --out OUT/original
// Ours:
//   cargo run --release -p bb-app --features ui --example frame_scenes -- OUT/ours \
//       --screen 2560x1440 --scenes parity/frames/escort.nut

::World.State.m.IsAutosaving = false;
::Math.seedRandom(20261013);

local mode = ::Settings.getVideoMode();
::logInfo("PROBE video mode = " + mode.Width + "x" + mode.Height + " ui " + mode.UIScale + " scene " + mode.SceneScale);

::probeSteps <- [
	{ scene = null, shot = "intro", until = null, click = "Huzzah!",
		clicked = "!Screens.WorldEventScreen.isVisible()" },
	{ scene = "before", shot = "escort_before", until = null, click = null },
	{ scene = "escort", shot = null, until = "rode", click = null },
	{ scene = "state", shot = "escort_ride", until = null, click = null },
	{ scene = "cancel", shot = "escort_after", until = null, click = null },
	{ scene = "state", shot = null, until = null, click = null }
];

// The company's look: visible or not, its sprites' Visible and alpha, where it is
// against the caravan, the camera.
local function look()
{
	local player = ::World.State.getPlayer();
	local text = "player isVisible " + player.isVisible() + " pos " + player.getPos().X + "," + player.getPos().Y;
	text += " sprites";

	foreach( name in [ "banner", "base", "body", "bodyUp", "combat", "lighting", "zoom_banner" ] )
	{
		if (player.hasSprite(name))
		{
			local s = player.getSprite(name);
			text += " " + name + "=" + s.Visible + "/" + s.Alpha;
		}
	}

	if ("probeContract" in ::getroottable())
	{
		local c = ::probeContract.m.Caravan;

		if (c != null && !c.isNull() && c.isAlive())
		{
			text += "; caravan isVisible " + c.isVisible() + " pos " + c.getPos().X + "," + c.getPos().Y;
		}
		else
		{
			text += "; no caravan";
		}
	}

	local escorted = ::World.State.getEscortedEntity();
	text += "; escorted " + (escorted != null && !escorted.isNull());
	local cam = ::World.getCamera();
	text += "; camera " + cam.getPos().X + "," + cam.getPos().Y + " zoom " + cam.Zoom;
	text += "; speed " + ::World.getSpeedMult() + " paused " + ::World.State.isPaused();
	return text;
}

::World.State.m.WorldScreen.probeScene <- function ( _scene )
{
	local state = ::World.State;
	local player = state.getPlayer();
	local note = "";
	::UI.setCursorHardware(true);
	::Cursor.setPosition(40, 20);

	if (_scene == "steps")
	{
		return ::probeSteps;
	}
	else if (_scene == "before")
	{
		state.setPause(true);
		::World.getCamera().Zoom = 1.0;
		::World.getCamera().setPos(player.getPos());
		note = look();
	}
	else if (_scene == "escort")
	{
		// The first settlement faction (by id) whose escort contract finds a destination,
		// as escort_caravan_action.onExecute makes it.
		local factions = ::World.FactionManager.getFactionsOfType(::Const.FactionType.Settlement).slice(0);
		factions.sort(function ( _a, _b ) { return _a.getID() <=> _b.getID(); });
		local contract;

		foreach( f in factions )
		{
			if (f.getSettlements().len() == 0)
			{
				continue;
			}

			local c = ::new("scripts/contracts/contracts/escort_caravan_contract");
			c.setFaction(f.getID());
			c.setEmployerID(f.getRandomCharacter().getID());
			c.setHome(f.getSettlements()[0]);
			c.setOrigin(f.getSettlements()[0]);
			c.setup();

			if (c.isValid())
			{
				contract = c;
				break;
			}
		}

		if (contract == null)
		{
			note = "no settlement has an escort contract";
		}
		else
		{
			::probeContract <- contract;
			::World.Contracts.addContract(contract);
			// The company at the caravan's town, as when it took the contract there.
			player.setPos(contract.m.Home.getPos());
			contract.start();
			contract.setState("Running");
			state.setPause(false);
			note = contract.m.Home.getName() + " -> " + contract.m.Destination.getName() + "; active " + (::World.Contracts.getActiveContract() == contract) + "; " + look();
		}
	}
	else if (_scene == "rode")
	{
		// Six seconds of riding along, a line of state each second, then paused (a shot
		// holds still).
		if (!("probeRideStart" in ::getroottable()))
		{
			::probeRideStart <- ::Time.getRealTimeF();
		}

		local t = ::Time.getRealTimeF() - ::probeRideStart;

		if (!("probeRideNext" in ::getroottable()))
		{
			::probeRideNext <- 1.0;
		}

		if (t >= ::probeRideNext)
		{
			::probeRideNext += 1.0;
			::logInfo("PROBE ride " + t + " = " + look());
		}

		if (t < 6.0)
		{
			return false;
		}

		state.setPause(true);
		return true;
	}
	else if (_scene == "state")
	{
		note = look();
	}
	else if (_scene == "cancel")
	{
		::World.Contracts.removeContract(::probeContract);
		state.setPause(true);
		::World.getCamera().setPos(player.getPos());
		note = look();
	}

	if (_scene != "rode")
	{
		::logInfo("PROBE scene " + _scene + " = " + note);
	}

	return true;
};
::logInfo("PROBE sq: escort scenes ready");
