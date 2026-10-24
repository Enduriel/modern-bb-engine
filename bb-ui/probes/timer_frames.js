// When timers run relative to the game's frames, in the original (menu mode). Prepend
// timer_frames_core.js: `cat timer_frames_core.js timer_frames.js > probe.js`. The
// scripts (turnbar_race.nut, whose `tbTick` this answers) call it once a frame.
var TimerFramesSent = false;
BbProbeScreen.prototype.tbTick = function (_frame)
{
	TimerFrames.tick(_frame);
	if (TimerFrames.isDone() && !TimerFramesSent)
	{
		TimerFramesSent = true;
		BbProbe.send('timer_frames', TimerFrames.results());
		BbProbe.done();
	}
};

BbProbe.run = function ()
{
	SQ.call(BbProbe.handle, 'tbStart');
};
