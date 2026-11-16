// mode: campaign seed=rust-engine-test
// World.setSpeedMult and the virtual speed: whether it goes through a script's ::Time.setVirtualSpeed.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
local setV = ::Time.setVirtualSpeed, calls = [];
local function state()
{
	return "world " + ::World.getSpeedMult() + " virtual " + ::Time.getVirtualSpeed() + " calls " + calls.len()
		+ (calls.len() ? " arg " + calls[0] : "");
}
p("start", state);

// A wrapper that passes the speed on, doubled.
::Time.setVirtualSpeed = function ( _v ) { calls.push(_v); setV.call(::Time, _v * 2); };
::World.setSpeedMult(3.0);
p("wrapper passing on x2, setSpeedMult 3", state);
calls.clear();
::Time.setVirtualSpeed(1.5);
p("wrapper passing on x2, setVirtualSpeed 1.5", state);

// A wrapper that swallows the call.
calls.clear();
::Time.setVirtualSpeed = function ( _v ) { calls.push(_v); };
::World.setSpeedMult(0.5);
p("wrapper swallowing, setSpeedMult 0.5", state);

// A wrapper that raises.
calls.clear();
::Time.setVirtualSpeed = function ( _v ) { calls.push(_v); throw "probe raises"; };
p("wrapper raising, setSpeedMult 4", @() ::World.setSpeedMult(4.0));
p("wrapper raising, after", state);

// The native back.
calls.clear();
::Time.setVirtualSpeed = setV;
::World.setSpeedMult(2.0);
p("native, setSpeedMult 2", state);
::World.setSpeedMult(1.0);
p("native, setSpeedMult 1", state);
