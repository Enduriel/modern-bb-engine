// The original's driver for world.nut (UI payload goldens): once the intro
// event is up, 3 s, then each step with 3 s after it, as bb-ui's world goldens pace
// theirs. ES5 only.
BbProbe.run = function ()
{
	var handle = function () { return Screens.WorldScreen.mSQHandle; };
	var step = function ()
	{
		SQ.call(handle(), 'probePayloadStep', null, function (result)
		{
			if (result === 'next')
			{
				setTimeout(step, 3000);
			}
			else
			{
				BbProbe.done();
			}
		});
	};
	var waited = 0;
	var poll = function ()
	{
		SQ.call(handle(), 'probePayloadReady', null, function (ready)
		{
			if (ready === true)
			{
				setTimeout(step, 3000);
				return;
			}
			waited += 500;
			if (waited > 120000)
			{
				console.error('PROBE_FAIL (js) no intro event after 120 s');
				BbProbe.done();
				return;
			}
			setTimeout(poll, 500);
		});
	};
	poll();
};
