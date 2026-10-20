// The soak runner's player (bb-engine/examples/soak): the choices a player makes, made
// through the scripts' own handlers (the screens' JS→SQ callbacks, the world state's
// AutoAttack / AutoEnterLocation, the town's slots). Installed after the harness's
// autoplay, which it wraps. Every function returns a short note for the run's record
// (or "" for nothing done); the autoplay's own choices are queued with `__soakNote` and
// drained by the runner.

::__soakSeed <- @SEED@;
::__soakNotes <- [];
::__soakDest <- null;

// Its own random stream (a 31-bit LCG), so its choices don't draw from the game's.
::__soakRand <- function( _n )
{
	::__soakSeed = (::__soakSeed * 1103515245 + 12345) & 0x7fffffff;
	return _n <= 1 ? 0 : (::__soakSeed >> 8) % _n;
};

::__soakNote <- function( _s )
{
	if (::__soakNotes.len() < 200) ::__soakNotes.push(_s);
};

::__soakDrain <- function()
{
	local s = "";
	foreach (i, n in ::__soakNotes) s += (i == 0 ? "" : "\x1e") + n;
	::__soakNotes = [];
	return s;
};

::__soakInTactical <- function()
{
	return ("State" in ::Tactical) && ::Tactical.State != null;
};

::__soakInWorld <- function()
{
	return ::RootState.get("WorldState") != null && ("State" in ::World) && ::World.State != null
		&& ::World.State.getPlayer() != null && !::LoadingScreen.isVisible();
};

// The event (or contract) on the event screen, and how many options its screen has.
::__soakEventOf <- function( _screen )
{
	try
	{
		local ev = _screen.m.IsContract ? ::World.Contracts.m.LastShown : ::World.Events.m.ActiveEvent;
		if (ev != null && ev.m.ActiveScreen != null) return [ev, ev.m.ActiveScreen.Options.len(), ev.getID() + "/" + ev.m.ActiveScreen.ID];
	}
	catch (e) {}
	return [null, 1, "?"];
};

::__soakAutoplayBase <- ::__autoplay;
::__autoplay = function()
{
	if (::__soakInTactical())
	{
		local st = ::Tactical.State;
		// When every enemy flees, the battle asks whether it's over: "It's over".
		local d = st.m.TacticalDialogScreen;
		if (st.m.IsEnemyRetreatDialogShown && d != null && d.isVisible() && !d.isAnimating())
		{
			st.tactical_retreat_screen_onYesPressed();
			::__soakNote("battle: the enemy flees, it's over");
			return;
		}
		// How it ended, once, before the result screen is left.
		if (st.isBattleEnded() && !("__soakEnded" in st.m))
		{
			st.m.__soakEnded <- true;
			local retreated = 0;
			try { retreated = ::Tactical.getRetreatRoster().getSize(); } catch (e) {}
			::__soakNote("battle ended: result " + ::Tactical.Entities.getCombatResult() + ", on the field: "
				+ ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Player).len() + " of the company, "
				+ ::Tactical.Entities.getHostilesNum() + " hostile; " + retreated + " retreated, fleeing " + st.m.IsFleeing);
		}
		::__soakAutoplayBase();
		// The brothers the autoplay handed to the AI stay and fight: in the player's
		// faction the AI's retreat has no odds check (it is the player's auto-retreat), so
		// they would all walk off the field at once.
		if (!st.isBattleEnded())
			foreach (bro in ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Player))
				if ("__autoplayHanded" in bro.m)
				{
					local a = bro.getAIAgent();
					if (a != null) a.getProperties().BehaviorMult[::Const.AI.Behavior.ID.Retreat] = 0.0;
				}
		return;
	}
	if (!("State" in ::World) || ::World.State == null) return;
	local state = ::World.State;
	local screen = state.getEventScreen();
	if (screen != null && screen.isVisible() && !screen.isAnimating())
	{
		local ev = ::__soakEventOf(screen);
		// A contract's first option takes it on (its task, its terms): mostly that, so
		// contracts get taken; an event's options are all equally likely.
		local pick = screen.m.IsContract && ::__soakRand(10) < 7 ? 0 : ::__soakRand(ev[1]);
		::__soakNote((screen.m.IsContract ? "contract " : "event ") + ev[2] + ": option " + pick + " of " + ev[1]);
		screen.onButtonPressed(pick);
	}
	// Engage once per offer, as a player clicks once.
	local dialog = state.m.CombatDialog;
	if (dialog == null || !dialog.isVisible()) ::__autoplayEngaged = false;
	else if (::__autoplayEngage && !::__autoplayEngaged && !dialog.isAnimating())
	{
		::__autoplayEngaged = true;
		::__soakNote("engage");
		dialog.onEngageButtonPressed();
	}
};

::__soakNavSettings <- function()
{
	local nav = ::World.getNavigator().createSettings();
	nav.ActionPointCosts = ::Const.World.TerrainTypeNavCost;
	nav.RoadMult = 1.0 / ::Const.World.MovementSettings.RoadMult;
	return nav;
};

// March to `_e` (a location to enter, or a party to attack), as a click on it does.
::__soakGoTo <- function( _e )
{
	local ws = ::World.State, p = ws.getPlayer();
	if (_e.isParty())
	{
		if (!_e.isHiddenToPlayer()) { ws.m.AutoAttack = ::WeakTableRef(_e); return true; }
	}
	local path = ::World.getNavigator().findPath(p.getTile(), _e.getTile(), ::__soakNavSettings(), 0);
	if (path.isEmpty()) return false;
	p.setPath(path);
	if (!_e.isParty()) ws.m.AutoEnterLocation = ::WeakTableRef(_e);
	return true;
};

// A settlement to visit: one of the six nearest friendly ones, or now and then any.
::__soakTravel <- function()
{
	local ws = ::World.State, p = ws.getPlayer();
	local towns = [];
	foreach (t in ::World.EntityManager.getSettlements())
		if (t != null && t.isAlive() && t.isEnterable() && t.isAlliedWithPlayer() && !t.getTile().isSameTileAs(p.getTile()))
			towns.push([t.getDistanceTo(p), t]);
	if (towns.len() == 0) return "";
	towns.sort(function( _a, _b ) { return _a[0] <=> _b[0]; });
	local pool = ::__soakRand(10) < 7 ? ::Math.min(6, towns.len()) : towns.len();
	for (local tries = 0; tries < 6; tries++)
	{
		local t = towns[::__soakRand(pool)][1];
		if (::__soakGoTo(t))
		{
			::__soakDest = ::WeakTableRef(t);
			return "travel to " + t.getName() + " (distance " + t.getDistanceTo(p).tointeger() + ")";
		}
	}
	return "";
};

// Whether the company is on its way somewhere (a path, a place to enter or a party to
// attack).
::__soakBusy <- function()
{
	local ws = ::World.State, p = ws.getPlayer();
	return p.hasPath() || (ws.m.AutoEnterLocation != null && !ws.m.AutoEnterLocation.isNull())
		|| (ws.m.AutoAttack != null && !ws.m.AutoAttack.isNull() && ws.m.AutoAttack.isAlive());
};

// A hostile party in sight and clearly weaker than the company, within `_radius` (the AI
// fights for the company, and not as well as a player).
::__soakHunt <- function( _radius )
{
	local ws = ::World.State, p = ws.getPlayer();
	local best = null, bestD = 0;
	foreach (e in ::World.getAllEntitiesAtPos(p.getPos(), _radius))
	{
		if (!e.isParty() || e.getID() == p.getID() || e.isAlliedWithPlayer() || !e.isAttackable() || e.isHiddenToPlayer()) continue;
		if (e.getTroops().len() == 0 || e.getVisibilityMult() == 0) continue;
		if (e.getStrength() > p.getStrength() * 0.6) continue;
		local d = e.getDistanceTo(p);
		if (best == null || d < bestD) { best = e; bestD = d; }
	}
	if (best == null) return "";
	ws.m.AutoAttack = ::WeakTableRef(best);
	return "hunt " + best.getName() + " (" + best.getTroops().len() + " troops, distance " + bestD.tointeger() + ")";
};

// Go after the active contract's target: its destination (a place or a party), or home.
::__soakPursue <- function()
{
	local c = ::World.Contracts.getActiveContract();
	if (c == null) return "";
	local unref = function( _r )
	{
		if (_r == null) return null;
		try { return _r.isNull() ? null : _r.get(); } catch (e) {}
		return _r;
	};
	local target = null;
	if ("Destination" in c.m) target = unref(c.m.Destination);
	if (target != null && (!("isAlive" in target) || !target.isAlive())) target = null;
	local what = "destination";
	if (target == null) { target = unref(c.m.Home); what = "home"; }
	if (target == null) return "contract " + c.getType() + ": nowhere to go";
	if (::World.State.getPlayer().getTile().isSameTileAs(target.getTile())) return "";
	if (!::__soakGoTo(target)) return "contract " + c.getType() + ": no path to its " + what;
	return "contract " + c.getType() + ": to its " + what + " " + target.getName();
};

::__soakBuilding <- function( _town, _id )
{
	foreach (i, b in _town.m.Buildings) if (b != null && b.getID() == _id) return i;
	return null;
};

::__soakStash <- function()
{
	return ("Stash" in ::getroottable()) && ::Stash != null ? ::Stash : ::World.Assets.getStash();
};

// One step of a town visit, through the town screen's own handlers.
::__soakTownStep <- function( _step )
{
	local ws = ::World.State, town = ws.getCurrentTown(), ts = ws.m.WorldTownScreen;
	if (town == null || !ts.isVisible()) return "";
	if (ts.isAnimating()) return "";
	if (_step == "contract")
	{
		if (::World.Contracts.getActiveContract() != null) return "";
		local cs = town.getContracts();
		if (cs.len() == 0) return town.getName() + ": no contracts";
		local c = cs[::__soakRand(cs.len())];
		::World.Contracts.showContract(c);
		return town.getName() + ": contract offered " + c.getType() + "#" + c.getID() + " (" + c.getName() + ")";
	}
	if (_step == "market")
	{
		local idx = ::__soakBuilding(town, "building.marketplace");
		if (idx == null) return town.getName() + ": no marketplace";
		local market = town.m.Buildings[idx];
		ts.onSlotClicked(idx);
		local mod = ts.getShopDialogModule(), stash = ::__soakStash(), s = town.getName() + ": market";
		local own = [];
		foreach (i, it in stash.getItems()) if (it != null) own.push(i);
		if (own.len() != 0)
		{
			local i = own[::__soakRand(own.len())];
			local name = stash.getItemAtIndex(i).item.getName();
			local r = mod.onSwapItem([i, "world-town-screen-shop-dialog-module.stash", null, "world-town-screen-shop-dialog-module.shop"]);
			s += ", sold " + name + (r == null ? " (refused)" : "");
		}
		local goods = [];
		foreach (i, it in market.getStash().getItems())
			if (it != null && it.getBuyPrice() <= ::World.Assets.getMoney()) goods.push(i);
		if (goods.len() != 0 && stash.hasEmptySlot())
		{
			local i = goods[::__soakRand(goods.len())];
			local name = market.getStash().getItemAtIndex(i).item.getName();
			local r = mod.onSwapItem([i, "world-town-screen-shop-dialog-module.shop", null, "world-town-screen-shop-dialog-module.stash"]);
			s += ", bought " + name + (r == null ? " (refused)" : "");
		}
		mod.onLeaveButtonPressed();
		return s;
	}
	if (_step == "hire")
	{
		if (::World.getPlayerRoster().getSize() >= ::World.Assets.getBrothersMax()) return town.getName() + ": roster full";
		local idx = ::__soakBuilding(town, "building.crowd");
		if (idx == null) return town.getName() + ": no crowd";
		ts.onSlotClicked(idx);
		local mod = ts.getHireDialogModule();
		local roster = ::World.getRoster(town.getID()).getAll();
		local s = town.getName() + ": no one to hire";
		if (roster.len() != 0)
		{
			local e = roster[::__soakRand(roster.len())];
			local name = e.getName();
			local r = mod.onHireRosterEntry(e.getID());
			s = town.getName() + ": hire " + name + " -> " + (r.Result == 0 ? "hired" : "refused " + r.Result);
		}
		mod.onLeaveButtonPressed();
		return s;
	}
	if (_step == "leave")
	{
		ws.m.MenuStack.pop();
		return "";
	}
	return "";
};

// Keep the company going: crowns, provisions, at least six brothers, the badly wounded
// healed. What it did, for the record ("" for nothing).
::__soakKeepAlive <- function()
{
	local a = ::World.Assets, s = "";
	if (a.getMoney() < 600) { a.addMoney(1500); s += " crowns+1500"; }
	if (a.getFood() < 25)
	{
		local n = 0;
		for (local i = 0; i < 4; i++)
			if (a.getStash().hasEmptySlot()) { a.getStash().add(::new("scripts/items/supplies/bread_item")); n++; }
		if ("updateFood" in a) a.updateFood();
		s += " bread+" + n;
	}
	if (a.getMedicine() < 10) { a.addMedicine(30); s += " medicine+30"; }
	if (a.getAmmo() < 20) { a.addAmmo(50); s += " ammo+50"; }
	if (a.getArmorParts() < 10) { a.addArmorParts(30); s += " tools+30"; }
	local roster = ::World.getPlayerRoster();
	local made = 0;
	while (roster.getSize() < 6 && roster.getSize() < a.getBrothersMax())
	{
		local bro = roster.create("scripts/entity/tactical/player");
		bro.m.HireTime = ::Time.getVirtualTimeF();
		bro.setStartValuesEx(["companion_1h_background"]);
		made++;
	}
	if (made != 0) s += " recruits+" + made;
	local healed = 0;
	foreach (bro in roster.getAll())
		if (bro.getHitpoints() < bro.getHitpointsMax() * 0.5) { bro.setHitpoints(bro.getHitpointsMax()); healed++; }
	if (healed != 0) s += " healed " + healed;
	return s;
};

// What must survive a save and load: the clock, money, the company and the stash.
::__soakInvariants <- function()
{
	local t = ::World.getTime();
	local s = "day=" + t.Days + " money=" + ::World.Assets.getMoney() + " roster=" + ::World.getPlayerRoster().getSize();
	local names = [];
	foreach (b in ::World.getPlayerRoster().getAll())
		names.push(b.getName() + ":" + b.getHitpoints() + "/" + b.getHitpointsMax() + ":xp" + b.getXP());
	names.sort();
	foreach (n in names) s += " " + n;
	local items = [];
	foreach (it in ::World.Assets.getStash().getItems()) if (it != null) items.push(it.getID());
	items.sort();
	s += " stash=" + items.len();
	foreach (i in items) s += "," + i;
	return s;
};

// What the game has up, for the record of a stall and the heartbeat.
::__soakState <- function( _detail )
{
	local s = "";
	try
	{
		if (::__soakInTactical())
		{
			local st = ::Tactical.State, bar = ::Tactical.TurnSequenceBar, a = bar.getActiveEntity();
			s = "battle round " + bar.getCurrentRound() + ", ended " + st.isBattleEnded();
			if (a != null) s += ", active " + a.getName() + " (" + a.getID() + ") hp " + a.getHitpoints() + " AP " + a.getActionPoints();
			if (_detail)
			{
				local agent = a == null ? null : a.getAIAgent();
				if (agent != null) s += ", agent " + agent.ClassName + " finished " + agent.isFinished() + " evaluating " + agent.isEvaluating();
				s += ", results " + (st.m.TacticalCombatResultScreen != null && st.m.TacticalCombatResultScreen.isVisible())
					+ ", menu backsteps " + st.m.MenuStack.hasBacksteps() + ", game paused " + st.m.IsGamePaused
					+ ", input locked " + st.m.IsInputLocked + ", action state " + st.m.CurrentActionState
					+ ", tactical dialog " + st.m.TacticalDialogScreen.isVisible() + ", virtual speed " + ::Time.getVirtualSpeed();
				local units = 0, hp = 0;
				foreach (faction in ::Tactical.Entities.getAllInstances())
					foreach (e in faction) if (e != null && e.isAlive()) { units++; hp += e.getHitpoints(); }
				s += ", units " + units + " hp " + hp;
			}
			return s;
		}
		if (::RootState.get("WorldState") == null) return "main menu, loading " + ::LoadingScreen.isVisible();
		if (!("State" in ::World) || ::World.State == null || ::World.State.getPlayer() == null) return "world (none), loading " + ::LoadingScreen.isVisible();
		local w = ::World.State, t = ::World.getTime(), ev = w.getEventScreen();
		s = "world day " + t.Days + " time " + t.Time + ", money " + ::World.Assets.getMoney() + ", roster " + ::World.getPlayerRoster().getSize()
			+ ", paused " + w.isPaused() + ", town " + w.m.WorldTownScreen.isVisible()
			+ ", event " + (ev != null && ev.isVisible() ? ::__soakEventOf(ev)[2] : "none")
			+ ", loading " + ::LoadingScreen.isVisible() + ", game over " + w.m.IsGameOver;
		if (_detail)
		{
			local c = ::World.Contracts.getActiveContract();
			s += ", speed " + ::World.getSpeedMult() + ", combat dialog " + (w.m.CombatDialog != null && w.m.CombatDialog.isVisible())
				+ ", menu stack " + w.m.MenuStack.hasBacksteps() + ", path " + w.getPlayer().hasPath()
				+ ", auto attack " + (w.m.AutoAttack != null && !w.m.AutoAttack.isNull())
				+ ", auto enter " + (w.m.AutoEnterLocation != null && !w.m.AutoEnterLocation.isNull())
				+ ", contract " + (c == null ? "none" : c.getType()) + ", camping " + ::World.Assets.isCamping();
			if ("MSU" in ::getroottable()) try { s += ", MSU popup " + ::MSU.Popup.isVisible(); } catch (e) {}
		}
	}
	catch (e) { s += " (" + e + ")"; }
	return s;
};
