// Frame parity scenes (the original's side): after the intro event, each scene is set up
// by the scripts (scenes.nut), given time to settle, and screenshotted by the runner.
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

var bbScenes = ['world'];

function bbNextScene(index)
{
	if (index >= bbScenes.length)
	{
		BbProbe.done();
		return;
	}
	var scene = bbScenes[index];
	SQ.call(Screens.WorldScreen.mSQHandle, 'probeScene', scene);
	// Time for the camera, the sprites' animations and the page to settle.
	setTimeout(function ()
	{
		BbProbe.send('page_' + scene, window.innerWidth + 'x' + window.innerHeight + ' dpr ' + window.devicePixelRatio);
		BbProbe.shot(scene, function (error)
		{
			if (error !== null)
			{
				console.error('PROBE js: shot ' + scene + ' failed: ' + error);
			}
			bbNextScene(index + 1);
		});
	}, 3000);
}

BbProbe.run = function ()
{
	var waited = 0;
	var poll = function ()
	{
		if (bbShowsText('Huzzah!'))
		{
			setTimeout(function ()
			{
				bbClickButton('Huzzah!');
				setTimeout(function () { bbNextScene(0); }, 3000);
			}, 3000);
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
