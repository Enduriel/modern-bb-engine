// The turn bar's first-slot race in the original (menu mode). Prepend
// turnbar_race_core.js: `cat turnbar_race_core.js turnbar_race.js > probe.js`. The
// scripts (turnbar_race.nut) call `tbTick` once a frame. ES5 only.
var TurnbarRaceSent = false;
BbProbeScreen.prototype.tbTick = function (_frame)
{
	TurnbarRace.tick(_frame);
	if (TurnbarRace.isDone() && !TurnbarRaceSent)
	{
		TurnbarRaceSent = true;
		BbProbe.send('turnbar_race', TurnbarRace.results());
		BbProbe.done();
	}
};

BbProbe.run = function ()
{
	// How the original reports an error thrown from a timer, as Velocity rethrows one
	// from an animation's `complete`.
	setTimeout(function () { throw new TypeError('TURNBAR_PROBE_UNCAUGHT'); }, 1);
	SQ.call(BbProbe.handle, 'tbStart');
};
