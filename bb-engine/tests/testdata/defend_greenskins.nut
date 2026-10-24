// A defend-settlement-from-greenskins contract taken and accepted as defend_settlement_greenskins_action
// and the town screen's accept do it, then its parties' distances to Home sampled over time.
// Shared with the original's probe (parity check of tests/world_defend_contract.rs).
::DG <- {
	Contract = null,
	Home = null,
	Start = 0.0,
	function start()
	{
		local factions = ::World.FactionManager.getFactionsOfType(::Const.FactionType.Settlement).slice(0);
		factions.sort(function ( _a, _b ) { return _a.getID() <=> _b.getID(); });
		local out = "none";
		foreach( f in factions )
		{
			if (f.getSettlements().len() == 0) continue;
			local home = f.getSettlements()[0];
			local c = ::new("scripts/contracts/contracts/defend_settlement_greenskins_contract");
			c.setFaction(f.getID());
			c.setEmployerID(f.getRandomCharacter().getID());
			c.setHome(home);
			c.setOrigin(home);
			if (!c.isValid()) continue;
			local orcs = c.getNearestLocationTo(home, ::World.FactionManager.getFactionOfType(::Const.FactionType.Orcs).getSettlements());
			local gobs = c.getNearestLocationTo(home, ::World.FactionManager.getFactionOfType(::Const.FactionType.Goblins).getSettlements());
			local dt = ::Math.min(orcs.getTile().getDistanceTo(home.getTile()), gobs.getTile().getDistanceTo(home.getTile()));
			if (dt < 10) continue;
			this.Contract = c;
			this.Home = home;
			::World.Contracts.addContract(c);
			::World.State.getPlayer().setPos(home.getPos());
			c.start();
			c.setState("Running");
			this.Start = ::Time.getVirtualTimeF();
			out = "home=" + home.getName() + " id=" + home.getID() + " tile=" + home.getTile().SquareCoords.X + "," + home.getTile().SquareCoords.Y
				+ " pos=" + home.getPos().X + "," + home.getPos().Y
				+ " orcs=" + orcs.getName() + "@" + orcs.getTile().getDistanceTo(home.getTile()) + "t/" + orcs.getDistanceTo(home)
				+ " gobs=" + gobs.getName() + "@" + gobs.getTile().getDistanceTo(home.getTile()) + "t/" + gobs.getDistanceTo(home)
				+ " goblins=" + c.m.Flags.get("IsGoblins") + " units=" + c.m.UnitsSpawned.len()
				+ " active=" + (::World.Contracts.getActiveContract() == c);
			foreach( id in c.m.UnitsSpawned )
			{
				local p = ::World.getEntityByID(id);
				local camp = ::World.FactionManager.getFaction(p.getFaction()).getNearestSettlement(home.getTile());
				out += " | party " + id + " " + p.getName() + " tile=" + p.getTile().SquareCoords.X + "," + p.getTile().SquareCoords.Y
					+ " pos=" + p.getPos().X + "," + p.getPos().Y + " baseDist=" + p.getDistanceTo(camp);
			}
			break;
		}
		return out;
	},
	function sample()
	{
		local c = this.Contract;
		local s = "t=" + (::Time.getVirtualTimeF() - this.Start) + " shown=" + c.m.Flags.get("IsEnemyHereDialogShown") + " state=" + c.m.ActiveState.ID;
		foreach( id in c.m.UnitsSpawned )
		{
			local p = ::World.getEntityByID(id);
			if (p == null || !p.isAlive()) { s += " | " + id + " gone"; continue; }
			s += " | " + id + " d=" + p.getDistanceTo(this.Home) + " dRef=" + p.getDistanceTo(c.m.Home)
				+ " tiles=" + p.getTile().getDistanceTo(this.Home.getTile()) + " pos=" + p.getPos().X + "," + p.getPos().Y;
		}
		return s;
	}
};
