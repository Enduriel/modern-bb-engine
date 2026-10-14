// Performance baselines in the original, menu mode: perf.py prepends benches.nut and original_common.nut.
// Logs the clock check, then each bench's minimum and median over five runs, in ms.

local function p( _k, _v )
{
	::logInfo("PROBE perf " + _k + " = " + _v);
}

p("mark menu", "t=" + ::Time.getExactTime());
p("real_time_s at menu", ::Time.getRealTimeF());

// Is getExactTime a live clock (it moves within one frame), unlike getRealTimeF?
{
	local e0 = ::Time.getExactTime(), r0 = ::Time.getRealTimeF(), s = 0;
	for( local i = 0; i < 2000000; i++ ) s += i;
	p("clock exact_moved_in_frame", ::Time.getExactTime() > e0);
	p("clock real_moved_in_frame", ::Time.getRealTimeF() > r0);
}

::__perfRunAll(::PerfBenches);
