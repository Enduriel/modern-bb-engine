// world_fights.nut: records the world map's auto-resolved fights (World.Combat, the
// script combat manager) for the world_fights example. Lines go through ::logInfo with
// the prefix "WF " so the example can pick them out. Safe to run again after a load.

::__wfDesc <- function( _p )
{
	if (_p == null) return "null";
	if (typeof _p == "instance") {
		if (_p.isNull()) return "dead-ref";
		_p = _p.get();
	}
	local strength = 0.0;
	foreach (t in _p.getTroops()) strength += t.Strength;
	return "#" + _p.getID() + " '" + _p.getName() + "' f" + _p.getFaction() + " troops " + _p.getTroops().len()
		+ " str " + strength + " cid " + _p.getCombatID() + (_p.isAlive() ? "" : " DEAD");
}

::__wfLog <- function( _s )
{
	::logInfo("WF day " + ::World.getTime().Days + " " + _s);
}

// Every live party: the units of every faction.
::__wfParties <- function()
{
	local out = [];
	foreach (f in ::World.FactionManager.getFactions()) {
		if (f == null) continue;
		foreach (u in f.getUnits()) {
			if (u == null) continue;
			if (typeof u == "instance") {
				if (u.isNull()) continue;
				u = u.get();
			}
			if (u.isAlive()) out.push(u);
		}
	}
	return out;
}

// Parties whose combat id names no combat the manager runs: frozen in place forever.
::__wfOrphans <- function()
{
	local ids = {};
	foreach (c in ::World.State.m.Combat.m.Combats) ids[c.ID] <- true;
	local out = [];
	foreach (p in ::__wfParties()) {
		if (p.getCombatID() != 0 && !(p.getCombatID() in ids)) out.push(p);
	}
	return out;
}

// The parties in a combat's faction lists, and the combatants' parties not in them.
::__wfCombatDesc <- function( _c )
{
	local listed = {};
	local refs = 0;
	foreach (list in _c.Factions) refs += list.len();
	local s = "combat " + _c.ID + " lists " + _c.Factions.len() + " refs " + refs + " combatants " + _c.Combatants.len() + ":";
	foreach (f, list in _c.Factions) {
		foreach (p in list) {
			if (p == null || p.isNull() || (p.getID() in listed)) continue;
			listed[p.getID()] <- true;
			s += " [" + ::__wfDesc(p) + "]";
		}
	}
	local unlisted = {};
	foreach (t in _c.Combatants) {
		if (t.Party == null || t.Party.isNull()) continue;
		local id = t.Party.getID();
		if (!(id in listed) && !(id in unlisted)) {
			unlisted[id] <- true;
			s += " UNLISTED-ATTACKER [" + ::__wfDesc(t.Party) + "]";
		}
	}
	return s;
}

::__wfInstall <- function()
{
	local cm = ::World.State.m.Combat;
	if ("__wfHooked" in cm) return;
	cm.__wfHooked <- true;
	::__wfSeen <- {};
	::__wfStats <- { Started = 0, Ended = 0, Throws = 0, Unlisted = 0 };

	local startCombat = cm.startCombat;
	cm.startCombat = function( _p1, _p2 ) {
		local before = this.m.Combats.len();
		::__wfLog("start " + ::__wfDesc(_p1) + " vs " + ::__wfDesc(_p2));
		try {
			startCombat(_p1, _p2);
		} catch (e) {
			++::__wfStats.Throws;
			::__wfLog("THROW in startCombat: " + e);
			throw e;
		}
	};
	local joinCombat = cm.joinCombat;
	cm.joinCombat = function( _combat, _party ) {
		try {
			joinCombat(_combat, _party);
		} catch (e) {
			++::__wfStats.Throws;
			::__wfLog("THROW in joinCombat (combat " + _combat.ID + ", " + _combat.Factions.len() + " faction slots, party " + ::__wfDesc(_party) + "): " + e);
			throw e;
		}
	};
	local update = cm.update;
	cm.update = function() {
		local before = {};
		foreach (c in this.m.Combats) {
			local ids = [];
			foreach (list in c.Factions) foreach (p in list) if (p != null && !p.isNull()) ids.push(p.getID());
			foreach (t in c.Combatants) if (t.Party != null && !t.Party.isNull() && ids.find(t.Party.getID()) == null) ids.push(t.Party.getID());
			before[c.ID] <- ids;
			if (!(c.ID in ::__wfSeen)) {
				::__wfSeen[c.ID] <- ::World.getTime().Days;
				++::__wfStats.Started;
				local d = ::__wfCombatDesc(c);
				if (d.find("UNLISTED") != null) ++::__wfStats.Unlisted;
				::__wfLog("first tick " + d);
			}
		}
		update();
		local now = {};
		foreach (c in this.m.Combats) now[c.ID] <- true;
		foreach (id, ids in before) {
			if (id in now) continue;
			++::__wfStats.Ended;
			local s = "end combat " + id + " (from day " + ::__wfSeen[id] + "):";
			foreach (pid in ids) {
				local e = ::World.getEntityByID(pid);
				s += " [" + (e == null ? "#" + pid + " gone" : ::__wfDesc(e)) + "]";
			}
			::__wfLog(s);
		}
	};
}

// One line on the state of the fights now.
::__wfStatus <- function()
{
	local fs = ::World.FactionManager.m.Factions;
	local orphans = ::__wfOrphans();
	local s = "status: roster " + ::World.getPlayerRoster().getSize() + (::World.State.m.IsGameOver ? ", GAME OVER" : "") + ", money " + ::World.Assets.getMoney() + ", " + fs.len() + " faction slots, " + ::World.State.m.Combat.m.Combats.len() + " combats, "
		+ orphans.len() + " orphaned parties in combat";
	foreach (o in orphans) s += " [" + ::__wfDesc(o) + "]";
	s += "; stats started " + ::__wfStats.Started + " ended " + ::__wfStats.Ended + " throws " + ::__wfStats.Throws
		+ " with-unlisted " + ::__wfStats.Unlisted;
	foreach (c in ::World.State.m.Combat.m.Combats) {
		if (c.ID in ::__wfSeen && ::World.getTime().Days - ::__wfSeen[c.ID] >= 1) s += "; LONG " + ::__wfCombatDesc(c);
	}
	return s;
}

// The factions, by id: type and name.
::__wfFactions <- function()
{
	local s = "";
	foreach (i, f in ::World.FactionManager.m.Factions) {
		if (f == null) continue;
		s += " " + i + ":" + f.getType() + ":" + f.getName();
	}
	return s;
}
