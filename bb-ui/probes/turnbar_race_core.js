// The turn bar's first-slot race, driven by the game's frames (shared by the probe for
// the original, turnbar_race.js, and our side's test). ES5 only.
//
// A turn bar module of its own (vanilla's TacticalScreenTurnSequenceBarModule, its
// calls to the scripts answered here) gets six units hidden from the player. Once the
// first has entered its slot fully, one frame's script call removes the third and adds
// a seventh (as the scripts do when a unit dies), which arms the module's 10 ms
// first-slot timer again; that timer selects the first unit anew (its slot and image
// resize again). A later frame's call removes the first unit (its turn ended), `delay`
// ms after the first call at the earliest. Each trial records whether the unit was
// selected anew before its removal, and whether that selection's image resize finished
// (its `complete` reads the image's placeholder, gone once the slot is removed:
// turnsequencebar_module.js "reading 'css'").
//
// `TurnbarRace.tick(frame)` is called once per game frame from the scripts;
// `TurnbarRace.done` turns true when every trial ran; `TurnbarRace.results()` is JSON.
var TurnbarRace = (function ()
{
	var R = { frame: 0, trials: [], errors: [], ticks: [], done: false };
	var DELAYS = [0, 0, 0, 0, 5, 10, 15, 20, 25, 30, 40, 60];
	var current = null;
	var next = 0;
	var base = 1000;

	function now()
	{
		return (window.performance && performance.now) ? performance.now() : Date.now();
	}

	var previousError = window.onerror;
	window.onerror = function (message, source, line, column)
	{
		R.errors.push({ trial: current ? current.index : -1, message: String(message), at: String(source).replace(/^.*\//, '') + ':' + line + ':' + column });
		if (typeof previousError === 'function')
		{
			return previousError.apply(this, arguments);
		}
		return false;
	};

	function count(trial, what, id)
	{
		var n = 0;
		for (var i = 0; i < trial.events.length; ++i)
		{
			if (trial.events[i][0] === what && trial.events[i][1] === id)
			{
				++n;
			}
		}
		return n;
	}

	function start(index)
	{
		var trial = { index: index, delay: DELAYS[index], events: [], phase: 'wait-first', t0: now(), data: {} };
		base += 100;
		trial.first = base + 1;
		var host = $('<div class="turnbar-race-host" style="position:absolute;left:0;top:0;width:1920px;height:1080px;pointer-events:none"/>');
		$('body').append(host);
		var m = new TacticalScreenTurnSequenceBarModule();
		var event = function (what, id)
		{
			trial.events.push([what, id, R.frame, Math.round(now() - trial.t0)]);
		};
		m.notifyBackendEntityEntersFirstSlot = function (id, callback)
		{
			event('enters', id);
			var data = trial.data[id];
			// The original answers a page call in a later task.
			setTimeout(function () { callback(data); }, 0);
		};
		m.notifyBackendEntityEnteredFirstSlot = function (id) { event('entered', id); };
		m.notifyBackendEntityEnteredFirstSlotFully = function (id) { event('fully', id); };
		m.notifyBackendEntityLeftFirstSlot = function (id) { event('left', id); };
		m.notifyBackendEntityRemoved = function (id) { event('removed', id); };
		// The panels for the first unit ask the scripts for its skills: not this race's.
		m.updateButtonBar = m.updateStatsPanel = m.updateEntitySkills = m.updateEntityStatusEffects = function () {};
		m.showEntitySkillbar = m.showEntityStatusEffectbar = m.showStatsPanel = function () {};
		m.create(host);
		m.show(0);
		trial.module = m;
		trial.host = host;
		for (var i = 1; i <= 6; ++i)
		{
			var data = { id: base + i, isHiddenToPlayer: true, isEnemy: true, imagePathFoW: Asset.IMAGE_ENTITY_SELECTION_ARROW };
			trial.data[base + i] = data;
			m.addEntity(data);
		}
		return trial;
	}

	function finish(trial)
	{
		var b = trial.first;
		R.trials.push({
			delay: trial.delay,
			// Frames and ms (from the call that armed the timer) of the removal.
			removedAfterFrames: trial.removeFrame - trial.armFrame,
			removedAfterMs: Math.round(trial.removeTime - trial.armTime),
			reselectedBeforeRemoval: trial.entersAtRemoval >= 2,
			enters: count(trial, 'enters', b),
			entered: count(trial, 'entered', b),
			fully: count(trial, 'fully', b),
			removed: count(trial, 'removed', b),
			events: trial.events.filter(function (e) { return e[2] >= trial.armFrame - 1; })
		});
		trial.module.destroy();
		trial.host.remove();
	}

	function tick(frame)
	{
		R.frame = frame;
		var t = now();
		R.ticks.push(Math.round(t * 10) / 10);
		if (R.done)
		{
			return;
		}
		if (current === null)
		{
			current = start(next);
			current.startFrame = frame;
			return;
		}
		var trial = current;
		var b = trial.first;
		if (trial.phase === 'wait-first')
		{
			if (count(trial, 'fully', b) >= 1)
			{
				trial.phase = 'settle';
				trial.settleUntil = t + 400;
			}
			else if (frame - trial.startFrame > 600)
			{
				trial.phase = 'failed';
				R.trials.push({ delay: trial.delay, failed: 'the first unit never entered its slot fully', events: trial.events });
				trial.module.destroy();
				trial.host.remove();
				current = null;
				++next;
			}
		}
		else if (trial.phase === 'settle')
		{
			if (t >= trial.settleUntil)
			{
				// A unit further down dies: the scripts remove its slot and add the next.
				trial.module.removeEntity(base + 3);
				var data = { id: base + 7, isHiddenToPlayer: true, isEnemy: true, imagePathFoW: Asset.IMAGE_ENTITY_SELECTION_ARROW };
				trial.data[base + 7] = data;
				trial.module.addEntity(data);
				trial.armFrame = frame;
				trial.armTime = t;
				trial.phase = 'armed';
			}
		}
		else if (trial.phase === 'armed')
		{
			if (t - trial.armTime >= trial.delay)
			{
				// The first unit's turn ends.
				trial.entersAtRemoval = count(trial, 'enters', b);
				trial.module.removeEntity(b);
				trial.removeFrame = frame;
				trial.removeTime = t;
				trial.phase = 'observe';
				trial.observeUntil = t + 1500;
			}
		}
		else if (trial.phase === 'observe')
		{
			if (t >= trial.observeUntil)
			{
				finish(trial);
				current = null;
				++next;
			}
		}
		if (current === null && next >= DELAYS.length)
		{
			R.done = true;
		}
	}

	function results()
	{
		var gaps = [];
		for (var i = 1; i < R.ticks.length; ++i)
		{
			gaps.push(R.ticks[i] - R.ticks[i - 1]);
		}
		gaps.sort(function (a, b) { return a - b; });
		var median = gaps.length > 0 ? gaps[Math.floor(gaps.length / 2)] : null;
		return JSON.stringify({ frames: R.ticks.length, frameMedianMs: median, frameMaxMs: gaps.length > 0 ? gaps[gaps.length - 1] : null, trials: R.trials, errors: R.errors });
	}

	return {
		tick: tick,
		results: results,
		isDone: function () { return R.done; }
	};
})();
