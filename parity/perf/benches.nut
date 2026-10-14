// Performance baselines: the same script work, timed in both engines.
// `perf.py` runs these in the original (timed with Time.getExactTime, its live clock);
// `bb-engine/examples/perf_baseline.rs` runs them in ours (timed from Rust). Each bench
// is one call of `Run`, sized to take tens to hundreds of milliseconds in the original.
// Menu-safe: no world needed (the world's are in benches_world.nut).

::PerfBenches <- [];

// Interpreter: integer arithmetic.
::PerfBenches.push({
	Name = "vm_int_loop",
	function Run()
	{
		local s = 0;

		for( local i = 0; i < 1000000; i++ )
		{
			s += i % 7;
		}

		return s;
	}

});
// Interpreter: float arithmetic (single precision, as game scripts run).
::PerfBenches.push({
	Name = "vm_float_loop",
	function Run()
	{
		local s = 0.0;

		for( local i = 0; i < 1000000; i++ )
		{
			s = s * 0.5 + i * 0.25;
		}

		return s;
	}

});
// Interpreter: script-to-script calls.
::PerfBenches.push({
	Name = "vm_calls",
	function Run()
	{
		local f = function ( _x )
		{
			return _x + 1;
		};
		local s = 0;

		for( local i = 0; i < 500000; i++ )
		{
			s = f(s);
		}

		return s;
	}

});
// Tables: create, write and read small tables.
::PerfBenches.push({
	Name = "tables",
	function Run()
	{
		local s = 0;

		for( local i = 0; i < 200000; i++ )
		{
			local t = {
				A = i,
				B = i * 2
			};
			t.C <- t.A + t.B;
			s += t.C;
		}

		return s;
	}

});
// Arrays: push, then sort random integers.
::PerfBenches.push({
	Name = "arrays",
	function Run()
	{
		local a = [];

		for( local i = 0; i < 200000; i++ )
		{
			a.push(i * 7 % 1000);
		}

		local b = [];
		::Math.seedRandom(1);

		for( local i = 0; i < 50000; i++ )
		{
			b.push(::Math.rand(0, 1000000));
		}

		b.sort();
		return a.len() + b[0];
	}

});
// Strings: concatenation, format and find.
::PerfBenches.push({
	Name = "strings",
	function Run()
	{
		local n = 0;

		for( local i = 0; i < 100000; i++ )
		{
			local s = "item" + i;
			s = format("%s:%d:%.2f", s, i, i * 0.5);

			if (s.find("99") != null)
			{
				n++;
			}
		}

		return n;
	}

});
// The class system: create vanilla items (inherit chains, onCreate).
::PerfBenches.push({
	Name = "bb_new_items",
	function Run()
	{
		local n = 0;

		for( local i = 0; i < 2000; i++ )
		{
			local item = ::new("scripts/items/weapons/arming_sword");
			n += item.getValue();
		}

		return n;
	}

});
// The class system: method calls on one object, through its delegate chain.
::PerfBenches.push({
	Name = "bb_method_calls",
	function Run()
	{
		local item = ::new("scripts/items/weapons/arming_sword");
		local n = 0;

		for( local i = 0; i < 200000; i++ )
		{
			n += item.getValue() + item.getName().len();
		}

		return n;
	}

});
// Global lookups from inside an object (`this.Math`), which miss every level of its
// delegate chain before the root table answers: vanilla's most common lookup.
::PerfBenches.push({
	Name = "bb_global_lookups",
	function Run()
	{
		local item = ::new("scripts/items/weapons/arming_sword");
		local f = function ( _n )
		{
			local s = 0;

			for( local i = 0; i < _n; i++ )
			{
				s += this.Math.min(i, 5) + this.Const.World.TerrainTypeNavCost.len();
			}

			return s;
		}.bindenv(item);
		return f(200000);
	}

});
// Natives: the PRNG.
::PerfBenches.push({
	Name = "native_rand",
	function Run()
	{
		::Math.seedRandom(2);
		local s = 0;

		for( local i = 0; i < 500000; i++ )
		{
			s += ::Math.rand(1, 100);
		}

		return s;
	}

});
// Natives: native object creation and member reads.
::PerfBenches.push({
	Name = "native_vec",
	function Run()
	{
		local s = 0.0;

		for( local i = 0; i < 200000; i++ )
		{
			local v = ::createVec(i, i * 2);
			s += v.X + v.Y;
		}

		return s;
	}

});
