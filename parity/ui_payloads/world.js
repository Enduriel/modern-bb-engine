// The original's driver for world.nut (UI payload goldens): once the intro
// event is up, 3 s, then each step with 3 s after it (after the ambush, once the
// player's first turn comes), as bb-ui's world goldens pace theirs. ES5 only.
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
			else if (result === 'waitPlayerTurn')
			{
				waitPlayerTurn(0);
			}
			else
			{
				BbProbe.done();
			}
		});
	};
	// Once one of the player's men is up, 3 s more, then the next step.
	var waitPlayerTurn = function (waitedMs)
	{
		SQ.call(handle(), 'probePayloadPlayerTurn', null, function (isPlayers)
		{
			if (isPlayers === true)
			{
				setTimeout(step, 3000);
			}
			else if (waitedMs > 180000)
			{
				console.error('PROBE_FAIL (js) no player turn after 180 s');
				BbProbe.done();
			}
			else
			{
				setTimeout(function () { waitPlayerTurn(waitedMs + 500); }, 500);
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
