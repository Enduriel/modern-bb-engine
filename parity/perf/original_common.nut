// Performance baselines in the original: what both drivers share (perf.py prepends it).

// Run each bench five times; log its minimum and median in milliseconds.
::__perfRunAll <- function ( _benches )
{
	foreach( b in _benches )
	{
		local times = [];

		for( local r = 0; r < 5; r++ )
		{
			local t0 = ::Time.getExactTime();
			b.Run();
			times.push((::Time.getExactTime() - t0) * 1000.0);
		}

		times.sort();
		::logInfo("PROBE perf bench " + b.Name + " min_ms = " + times[0]);
		::logInfo("PROBE perf bench " + b.Name + " median_ms = " + times[2]);
	}
};
