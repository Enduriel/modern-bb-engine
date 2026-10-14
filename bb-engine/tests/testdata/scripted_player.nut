// The scripted player of the draw-exact player battles (`combat_scenarios.rs`), the
// same script the captures ran in the original game (2026-10-14). It
// plays the brothers' turns through the entry points a player's input reaches: the
// tactical state's onMouseInput (clicks on tiles) and onKeyInput, the turn bar's
// buttons (skills, Wait, End Turn), and the inventory's onEquipBagItem.
//
// Each brother waits once in the first round. On its first turn it takes the item
// `::__botUnpack` out of its bag, if any. Then it uses its first usable attack on the
// nearest visible enemy (skills in `::__botPrefer` first), or walks up to it (twice
// at most), or ends its turn. It holds while anything is under way (the turn hasn't
// started, the agent is thinking, input is locked, a timed event is pending, the bar
// is locked, a move or skill is running).

// Skills the scripted player tries first, in order.
::__botPrefer <- [];
// An item each brother takes out of its bag on its first turn (through the inventory).
::__botUnpack <- null;
::__key <- function( _key, _state )
{
	return {
		key = _key, state = _state,
		getKey = function() { return this.key; },
		getState = function() { return this.state; },
		getModifier = function() { return 0; }
	};
};
// The 'C' key, pressed and released: opens the inventory, and closes it again.
::__pressC <- function()
{
	::Tactical.State.onKeyInput(::__key(13, 0));
	::Tactical.State.onKeyInput(::__key(13, 1));
};
// A mouse event, as the engine hands onMouseInput.
::__mouse <- function( _id, _state, _x, _y )
{
	return {
		id = _id, state = _state, x = _x, y = _y,
		getID = function() { return this.id; },
		getState = function() { return this.state; },
		getX = function() { return this.x; },
		getY = function() { return this.y; }
	};
};
// Click a tile: centre the camera on it, then move, press and release at the centre.
::__clickTile <- function( _tile )
{
	local st = ::Tactical.State;
	::Tactical.getCamera().setPos(_tile.Pos);
	local mode = ::Settings.getVideoMode();
	local x = mode.Width / 2, y = mode.Height / 2;
	st.onMouseInput(::__mouse(6, 0, x, y));
	st.onMouseInput(::__mouse(1, 0, x, y));
	st.onMouseInput(::__mouse(1, 1, x, y));
};
::__tileOrder <- function( _a, _b )
{
	if (_a.SquareCoords.X != _b.SquareCoords.X) return _a.SquareCoords.X < _b.SquareCoords.X ? -1 : 1;
	if (_a.SquareCoords.Y != _b.SquareCoords.Y) return _a.SquareCoords.Y < _b.SquareCoords.Y ? -1 : 1;
	return 0;
};
// The visible enemies, nearest first (ties by tile; ids differ between engines).
::__hostiles <- function( _e )
{
	local out = [];
	foreach (f in ::Tactical.Entities.getAllInstances())
		foreach (a in f)
			if (a.isAlive() && a.isPlacedOnMap() && !a.isAlliedWith(_e) && !a.isHiddenToPlayer()) out.push(a);
	local my = _e.getTile();
	out.sort(function( a, b ) {
		local da = my.getDistanceTo(a.getTile()), db = my.getDistanceTo(b.getTile());
		if (da != db) return da < db ? -1 : 1;
		return ::__tileOrder(a.getTile(), b.getTile());
	});
	return out;
};
::__bot <- { turn = "", moves = 0, acted = 0, waited = {}, why = "", unpacked = {}, opening = false };
::__botLog <- function( _e, _what ) { ::logInfo("RNG BOT n=" + ::__rng.n + " " + _e.getName() + "@" + _e.getTile().SquareCoords.X + "," + _e.getTile().SquareCoords.Y + " ap=" + _e.getActionPoints() + " " + _what); };
::__botTick <- function()
{
	if (!("State" in ::Tactical) || ::Tactical.State == null) return;
	local st = ::Tactical.State;
	if (st.isBattleEnded() || ::LoadingScreen.isVisible()) return;
	local bar = ::Tactical.TurnSequenceBar;
	local e = bar.getActiveEntity();
	if (e == null || !e.isPlayerControlled()) return;
	local why = "";
	if (!e.isTurnStarted()) why = "turn not started";
	else if (e.getAIAgent() != null && !e.getAIAgent().isFinished()) why = "agent thinking";
	else if (st.isInputLocked()) why = "input locked";
	// The bar ignores End Turn while a timed event (a hit landing, a sound) is pending.
	else if (::Time.hasEventScheduled(::TimeUnit.Virtual)) why = "event pending";
	else if (bar.m.IsLocked) why = "bar locked";
	else if (::Tactical.getNavigator().isTravelling(e)) why = "travelling";
	else if (st.m.CurrentActionState == ::Const.Tactical.ActionState.TravelPath || st.m.CurrentActionState == ::Const.Tactical.ActionState.ExecuteSkill) why = "action " + st.m.CurrentActionState;
	else if (st.isInCharacterScreen() && !::__bot.opening) why = "inventory closing";
	if (why != "")
	{
		if (::__bot.why != why) { ::__bot.why = why; ::__botLog(e, "hold: " + why); }
		return;
	}
	::__bot.why = "";
	local round = bar.getCurrentRound();
	local key = e.getName() + "@" + round;
	if (::__bot.turn != key) { ::__bot.turn = key; ::__bot.moves = 0; ::__bot.acted = 0; }
	if (::__botUnpack != null && !(e.getName() in ::__bot.unpacked))
	{
		local item = null;
		for (local i = 0; i < e.getItems().getUnlockedBagSlots(); i++)
		{
			local it = e.getItems().getItemAtBagSlot(i);
			if (it != null && it.getID() == ::__botUnpack) { item = it; break; }
		}
		if (item == null) { ::__bot.unpacked[e.getName()] <- true; }
		else
		{
			local cs = st.m.CharacterScreen;
			if (!::__bot.opening) { ::__bot.opening = true; ::__botLog(e, "open inventory"); ::__pressC(); return; }
			if (!cs.isVisible() || cs.isAnimating()) return;
			local r = cs.onEquipBagItem([e.getID(), item.getInstanceID(), null]);
			::__botLog(e, "equip " + item.getID() + ": " + ((typeof r == "table" && ("error" in r)) ? r.error : "ok"));
			::__bot.unpacked[e.getName()] <- true;
			::__bot.opening = false;
			::__pressC();
			return;
		}
	}
	if (++::__bot.acted > 12) { ::__botLog(e, "end turn (stuck)"); bar.onNextTurnButtonPressed(); return; }
	local enemies = ::__hostiles(e);
	if (round == 1 && !(e.getName() in ::__bot.waited) && e.isAbleToWait())
	{
		::__bot.waited[e.getName()] <- true;
		::__botLog(e, "wait");
		bar.onWaitTurnButtonPressed();
		return;
	}
	local actives = [];
	foreach (id in ::__botPrefer) { local sk = e.getSkills().getSkillByID(id); if (sk != null) actives.push(sk); }
	foreach (sk in e.getSkills().queryActives()) if (::__botPrefer.find(sk.getID()) == null) actives.push(sk);
	foreach (sk in actives)
	{
		if (!sk.isAttack() || !sk.isUsable() || !sk.isAffordable()) continue;
		foreach (en in enemies)
		{
			if (!sk.isUsableOn(en.getTile())) continue;
			::__botLog(e, "skill " + sk.getID() + " -> " + en.getName() + "@" + en.getTile().SquareCoords.X + "," + en.getTile().SquareCoords.Y);
			bar.onEntitySkillClicked({ entityId = e.getID(), skillId = sk.getID() });
			if (sk.isTargeted()) ::__clickTile(en.getTile());
			return;
		}
	}
	if (enemies.len() != 0 && ::__bot.moves < 2 && e.getActionPoints() >= 4)
	{
		::__bot.moves++;
		local target = enemies[0].getTile();
		local dest = null, best = 9999;
		for (local d = 0; d < 6; d++)
		{
			if (!target.hasNextTile(d)) continue;
			local t = target.getNextTile(d);
			if (!t.IsEmpty) continue;
			local dist = e.getTile().getDistanceTo(t);
			if (dist < best) { best = dist; dest = t; }
		}
		if (dest != null)
		{
			::__botLog(e, "move -> " + dest.SquareCoords.X + "," + dest.SquareCoords.Y);
			::__clickTile(dest);
			if (st.m.CurrentActionState == ::Const.Tactical.ActionState.ComputePath) ::__clickTile(dest);
			return;
		}
	}
	::__botLog(e, "end turn");
	bar.onNextTurnButtonPressed();
};

