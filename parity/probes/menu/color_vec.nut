// mode: menu
// color and vec natives.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
local c = ::createColor("#ff804020");
p("createColor R", @() c.R);
p("createColor G", @() c.G);
p("createColor B", @() c.B);
p("createColor A", @() c.A);
p("createColor short", @() ::createColor("#fff").R);
p("createColorRGB", @() ::createColorRGB(10, 20, 30).G);
p("createColorWhite A", @() ::createColorWhite(0.5).A);
local v = ::createVec(3, 4);
p("vec X", @() v.X);
p("vec Y", @() v.Y);
p("getVecDistance", @() ::getVecDistance(::createVec(0, 0), v));
p("getVecDir X", @() ::getVecDir(::createVec(0, 0), v).X);
p("getVecDir Y", @() ::getVecDir(::createVec(0, 0), v).Y);
p("vec add", @() (v + ::createVec(1, 1)).X);
// createColor edge cases: vanilla's 9-digit typos, `#` with 8 digits, malformed input.
foreach (s in ["fffffff00", "fffffffe0", "ffffffff0", "#ff804020", "ff804020", "#80402", "#xyz", "zz", "", " 0x10203040", "#0x112233"])
	p("createColor " + s + " RGBA", function() { local c = ::createColor(s); return c.R + "," + c.G + "," + c.B + "," + c.A; });
p("createColorRGB 0.5,1.0,0.25", function() { local c = ::createColorRGB(0.5, 1.0, 0.25); return c.R + "," + c.G + "," + c.B + "," + c.A; });
p("createColorRGB -0.1,2.0,1.001", function() { local c = ::createColorRGB(-0.1, 2.0, 1.001); return c.R + "," + c.G + "," + c.B + "," + c.A; });
