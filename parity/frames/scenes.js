// Frame parity scenes (the original's side): once the intro event shows, walk the
// scripts' ::probeSteps (scenes.nut). Each step's scene is set up by the scripts; its
// `until` is polled until true; after 3 s to settle, its shot is taken by the runner
// (BbProbe.shot, which answers once written), and its `click` button is clicked.
function bbShowsText(text)
{
	var walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, null, false);
	for (var node = walker.nextNode(); node !== null; node = walker.nextNode())
	{
		if (node.nodeValue.indexOf(text) >= 0 && node.parentNode.offsetParent !== null)
		{
			return true;
		}
	}
	return false;
}

function bbClickButton(label)
{
	var all = document.querySelectorAll('.ui-control');
	for (var i = 0; i < all.length; i++)
	{
		if (all[i].offsetParent !== null && all[i].textContent.replace(/^\s+|\s+$/g, '') === label)
		{
			$(all[i]).click();
			return;
		}
	}
	console.error('PROBE js: no visible button ' + label);
}

// The orientation overlays' click targets in a battle shot: per overlay its id, classes
// and box.
function bbSendOverlays(shot)
{
	if (shot.indexOf('battle_orientation') !== 0)
	{
		return;
	}
	var lines = [];
	$('.orientation-overlay-module .overlay').each(function ()
	{
		var r = this.getBoundingClientRect();
		var data = $(this).data('overlay');
		lines.push((data ? data.id : '?') + ' ' + this.className + ' ' + r.left + ' ' + r.top + ' ' + r.width + ' ' + r.height);
	});
	BbProbe.send('overlays_' + shot, lines.join('\n'));
}

function bbScene(name, then)
{
	SQ.call(Screens.WorldScreen.mSQHandle, 'probeScene', name, then);
}

// Poll scene `name` until it returns true (up to 180 s), then call `then`.
function bbUntil(name, then)
{
	var waited = 0;
	var poll = function ()
	{
		bbScene(name, function (done)
		{
			if (done === true)
			{
				then();
				return;
			}
			waited += 500;
			if (waited > 180000)
			{
				console.error('PROBE js: ' + name + ' never came true');
				then();
				return;
			}
			setTimeout(poll, 500);
		});
	};
	poll();
}

function bbStep(steps, index)
{
	if (index >= steps.length)
	{
		BbProbe.done();
		return;
	}
	var step = steps[index];
	var next = function () { bbStep(steps, index + 1); };
	var finish = function ()
	{
		if (step.click !== null && step.click !== undefined)
		{
			bbClickButton(step.click);
			setTimeout(next, 3000);
		}
		else
		{
			next();
		}
	};
	var shoot = function ()
	{
		// Time for the camera, the sprites' animations and the page to settle.
		setTimeout(function ()
		{
			if (step.shot === null || step.shot === undefined)
			{
				finish();
				return;
			}
			BbProbe.shot(step.shot, function (error)
			{
				if (error !== null)
				{
					console.error('PROBE js: shot ' + step.shot + ' failed: ' + error);
				}
				bbSendOverlays(step.shot);
				finish();
			});
		}, 3000);
	};
	var settle = function ()
	{
		if (step.until !== null && step.until !== undefined)
		{
			bbUntil(step.until, shoot);
		}
		else
		{
			shoot();
		}
	};
	if (step.scene !== null && step.scene !== undefined)
	{
		bbScene(step.scene, function () { settle(); });
	}
	else
	{
		settle();
	}
}

BbProbe.run = function ()
{
	var waited = 0;
	var poll = function ()
	{
		if (bbShowsText('Huzzah!'))
		{
			BbProbe.send('page', window.innerWidth + 'x' + window.innerHeight + ' dpr ' + window.devicePixelRatio);
			bbScene('steps', function (steps) { bbStep(steps, 0); });
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
	};
	poll();
};
