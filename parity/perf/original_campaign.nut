// Performance baselines in the original, campaign mode (seed rust-engine-test): perf.py prepends
// benches_world.nut and original_common.nut. Runs after world_state.init(). Logs the new
// campaign's time, the world benches, the frame rate on the running world map, then
// save and load times (save "bb_probe_perf", which perf.py deletes afterwards).

local function p( _k, _v )
{
	::logInfo("PROBE perf " + _k + " = " + _v);
}

p("new_campaign_s", ::Time.getExactTime() - ::__perfCampaignStart);
p("mark world", "t=" + ::Time.getExactTime());
::__perfRunAll(::PerfWorldBenches);

// The video mode the frame rate was measured at.
{
	local m = ::Settings.getVideoMode();
	foreach( k in ["Width", "Height", "ColorBits", "Fullscreen", "Borderless", "VSync", "UIScale", "SceneScale"] )
	{
		try { p("video " + k, m[k]); } catch( e ) {}
	}
}

::BbProbe.DeferDone = true;
::__perfFps <- [];

::__perfFinish <- function ()
{
	local fps = ::__perfFps;
	local sum = 0.0;
	foreach( f in fps ) sum += f;
	fps.sort();
	p("world fps samples", fps.len());
	if (fps.len() > 0)
	{
		p("world fps mean", sum / fps.len());
		p("world fps min", fps[0]);
		p("world fps median", fps[fps.len() / 2]);
	}

	p("mark save", "t=" + ::Time.getExactTime());
	local t0 = ::Time.getExactTime();
	::World.State.saveCampaign("bb_probe_perf");
	p("save_ms", (::Time.getExactTime() - t0) * 1000.0);
	t0 = ::Time.getExactTime();
	::World.State.loadCampaign("bb_probe_perf");
	p("load_ms", (::Time.getExactTime() - t0) * 1000.0);
	p("mark end", "t=" + ::Time.getExactTime());
	::BbProbe.done();
};

// Sample the frame rate every 0.5 s for 10 s, once the world runs.
::__perfSampleFps <- function ( _left )
{
	::__perfFps.push(::Time.getFPS());

	if (_left > 1)
	{
		::Time.scheduleEvent(::TimeUnit.Real, 500, function ( _d ) { ::__perfSampleFps(_left - 1); }, null);
	}
	else
	{
		::__perfFinish();
	}
};

// Wait for the intro event, dismiss it, run the world, settle 2 s, then sample.
::__perfWaitIntro <- function ( _tries )
{
	if (::World.State.m.EventScreen.isVisible())
	{
		::World.State.m.EventScreen.onButtonPressed(0);
		::World.State.setPause(false);
		p("mark world_running", "t=" + ::Time.getExactTime());
		::Time.scheduleEvent(::TimeUnit.Real, 2000, function ( _d ) { ::__perfSampleFps(20); }, null);
	}
	else if (_tries > 0)
	{
		::Time.scheduleEvent(::TimeUnit.Real, 250, function ( _d ) { ::__perfWaitIntro(_tries - 1); }, null);
	}
	else
	{
		p("world fps", "no intro event");
		::__perfFinish();
	}
};
::__perfWaitIntro(240);
