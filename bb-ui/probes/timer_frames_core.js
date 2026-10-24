// When the page's timers and animation frames run relative to the game's frames (shared
// by the probe for the original, timer_frames.js, and our side). ES5 only.
//
// The scripts call `TimerFrames.tick(frame)` once per game frame. On some frames the
// call arms timers (delays 0..50 ms), a chain of zero-delay timers, a chain of animation
// frames and a 5 ms interval; each records the last game frame whose call it followed
// and the ms since it was armed. Arming also happens from a timer of the page's own (a
// page task) and from an animation frame.
var TimerFrames = (function ()
{
	var R = { frame: 0, ticks: [], sets: [], done: false };
	var DELAYS = [0, 1, 5, 8, 10, 12, 15, 20, 25, 30, 40, 50];

	function now()
	{
		return (window.performance && performance.now) ? performance.now() : Date.now();
	}

	function arm(label)
	{
		var set = { label: label, frame: R.frame, timers: {}, chain: [], frames: [], interval: [] };
		var t0 = now();
		var at = function () { return [R.frame - set.frame, Math.round(now() - t0)]; };
		R.sets.push(set);
		DELAYS.forEach(function (d)
		{
			setTimeout(function () { set.timers[d] = at(); }, d);
		});
		var link = function ()
		{
			set.chain.push(at());
			if (set.chain.length < 6)
			{
				setTimeout(link, 0);
			}
		};
		setTimeout(link, 0);
		var frame = function ()
		{
			set.frames.push(at());
			if (set.frames.length < 4)
			{
				requestAnimationFrame(frame);
			}
		};
		requestAnimationFrame(frame);
		var interval = setInterval(function ()
		{
			set.interval.push(at());
			if (set.interval.length >= 10)
			{
				clearInterval(interval);
			}
		}, 5);
	}

	function tick(frame)
	{
		R.frame = frame;
		R.ticks.push(Math.round(now() * 10) / 10);
		if (frame === 30)
		{
			arm('script call');
		}
		else if (frame === 60)
		{
			setTimeout(function () { arm('page timer'); }, 0);
		}
		else if (frame === 90)
		{
			requestAnimationFrame(function () { arm('animation frame'); });
		}
		else if (frame >= 130)
		{
			R.done = true;
		}
	}

	function results()
	{
		var gaps = [];
		for (var i = 1; i < R.ticks.length; ++i)
		{
			gaps.push(Math.round((R.ticks[i] - R.ticks[i - 1]) * 10) / 10);
		}
		return JSON.stringify({ gaps: gaps.slice(25, 45), sets: R.sets });
	}

	return { tick: tick, results: results, isDone: function () { return R.done; } };
})();
