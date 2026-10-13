// UI bridge ground truth (JS side of bridge_order.nut). ES5 only. See README.md.
// Ordering markers go through console.error('PROBE js: ...') so they land in log.html in
// sequence with the SQ side's ::logInfo rows.
var BBProbe = { echoes: [], callbacks: [], scripts: [], xhr: [] };

function jsLog(msg) { console.error('PROBE js: ' + msg); }

BbProbeScreen.prototype.echo = function (_args)
{
	BBProbe.echoes.push({ argc: arguments.length, isUndefined: _args === undefined, type: typeof _args, json: JSON.stringify(_args), keys: (_args && typeof _args === 'object') ? Object.keys(_args) : null });
};
BbProbeScreen.prototype.pFn = function (_v)
{
	jsLog('pFn(' + _v + ') runs, calling SQ pCb');
	SQ.call(BbProbe.handle, 'pCb', _v);
	jsLog('pFn(' + _v + ') returns');
};
BbProbeScreen.prototype.pX = function () { jsLog('pX ran'); };

// A second screen, registered after registrationFinished, for the UI.connect test.
var BBProbeScreen2 = function () { this.mSQHandle = null; };
BBProbeScreen2.prototype.onConnection = function (_h)
{
	jsLog('Screen2.onConnection(' + JSON.stringify(_h) + ') runs; notifying SQ');
	this.mSQHandle = _h;
	SQ.call(_h, 'onScreenConnected');
};
BBProbeScreen2.prototype.mark = function (_v) { jsLog('Screen2.mark(' + _v + ')'); };

function describeHandle(h)
{
	return { type: typeof h, json: (function () { try { return JSON.stringify(h); } catch (e) { return 'ERR ' + e; } })() };
}

function xhrStatus(url)
{
	try
	{
		var x = new XMLHttpRequest();
		x.open('GET', url, false);
		x.send(null);
		BBProbe.xhr.push({ url: url, status: x.status, length: (x.responseText || '').length });
	}
	catch (e) { BBProbe.xhr.push({ url: url, error: String(e) }); }
}

BbProbe.run = function ()
{
	var h = BbProbe.handle;
	registerScreen('BBProbeScreen2', new BBProbeScreen2());

	// The engine object as the native side set it up.
	var engineInfo = {};
	Object.getOwnPropertyNames(window.engine).sort().forEach(function (k) {
		var d = Object.getOwnPropertyDescriptor(window.engine, k);
		engineInfo[k] = typeof window.engine[k] + (d && d.writable === false ? ' (readonly)' : '') + (d && d.get ? ' (getter)' : '');
	});
	BbProbe.send('engine', JSON.stringify({
		keys: engineInfo,
		isAttached: engine.isAttached,
		events: Object.keys(engine.events).sort(),
		compatMode: document.compatMode,
		userAgent: navigator.userAgent,
		viewport: [window.innerWidth, window.innerHeight, window.devicePixelRatio],
		activeRequestsAtStart: Object.keys(engine._ActiveRequests).length
	}));

	// Handles.
	var mm = Screens.MainMenuScreen;
	BbProbe.send('handles', JSON.stringify({
		probe: describeHandle(h),
		root: describeHandle(Screens.RootScreen.mSQHandle),
		versionText: $('.root-screen').text().slice(0, 80),
		mainMenu: describeHandle(mm.mSQHandle),
		mainMenuModule: describeHandle(mm.mMainMenuModule.mSQHandle),
		loadCampaignModule: describeHandle(mm.mLoadCampaignModule.mSQHandle),
		tooltip: describeHandle(Screens.TooltipScreen.mSQHandle),
		loading: describeHandle(Screens.LoadingScreen.mSQHandle),
		console: describeHandle(Screens.ConsoleScreen.mSQHandle)
	}));

	// JS -> SQ argument counts.
	SQ.call(h, 'types', 2.7);
	SQ.call(h, 'noArgs');
	SQ.call(h, 'oneArg');
	SQ.call(h, 'noArgs', [1]);
	engine.call('asyncCallSQ', h, 'twoArgs', 1, 2);
	engine.call('asyncCallSQ', h, 'oneArg', 5, undefined);

	// JS -> SQ ordering: does asyncCallSQ run SQ before the next JS statement?
	SQ.call(h, 'logNow');
	jsLog('after asyncCallSQ(logNow) statement');
	SQ.call(h, 'pM', null, function (r) { jsLog('callSQ(pM) callback, result ' + JSON.stringify(r)); });
	jsLog('after callSQ(pM) statement');

	// callSQ return values, and whether void calls resolve.
	var tick = 'same tick';
	['ret', 'retInt', 'retNull', 'retNone', 'throws', 'missingMethod'].forEach(function (m) {
		engine.call('callSQ', h, m).then(function (r) {
			BBProbe.callbacks.push({ method: m, tick: tick, argc: arguments.length, type: typeof r, json: JSON.stringify(r) });
		}, function (err) {
			BBProbe.callbacks.push({ method: m, tick: tick, rejected: JSON.stringify(err) });
		});
	});
	engine.call('asyncCallSQ', h, 'noArgs').then(function () { BBProbe.callbacks.push({ method: 'asyncCallSQ noArgs', resolved: true, argc: arguments.length }); });
	tick = 'later';

	// SQ -> JS values.
	SQ.call(h, 'pushEchoes');

	// Dynamic script execution order: a larger script, then a tiny one, default async.
	['ui/extern/xbbcode/xbbcode.js', 'ui/error_codes.js'].forEach(function (src) {
		var s = document.createElement('script');
		s.onload = function () { BBProbe.scripts.push(src); };
		s.src = 'coui://' + src;
		BBProbe.scripts.push('async default=' + s.async);
		document.head.appendChild(s);
	});

	// Resolver rules.
	xhrStatus('coui://data/gfx/fonts/cinzel/Cinzel-Regular.ttf');
	xhrStatus('coui://gfx/fonts/cinzel/Cinzel-Regular.ttf');
	xhrStatus('coui://gfx/fonts/arsenal/Arsenal-Regular.ttf');
	xhrStatus('coui://data/ui/main.html');
	xhrStatus('coui://UI/main.html');
	xhrStatus('coui://ui/MAIN.html');
	xhrStatus('coui://ui/main.html?reload=123');

	setTimeout(function ()
	{
		BBProbe.activeRequestsAtEnd = Object.keys(engine._ActiveRequests).length;
		BbProbe.send('echoes', JSON.stringify(BBProbe.echoes));
		BbProbe.send('callbacks', JSON.stringify(BBProbe.callbacks));
		BbProbe.send('scripts', JSON.stringify(BBProbe.scripts));
		BbProbe.send('xhr', JSON.stringify(BBProbe.xhr));
		BbProbe.send('requests', JSON.stringify({ atEnd: BBProbe.activeRequestsAtEnd }));
		BbProbe.done();
	}, 4000);
};
