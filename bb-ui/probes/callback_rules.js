// UI bridge ground truth, the page half of callback_rules.nut: each receiver called from
// a page task (a timer, as Modern Hooks calls `resumeOnInit` from a script's onload), its
// callback's argument recorded (or "no callback" after a while). ES5 only.
var BBProbeScreen3 = function () {};
BBProbeScreen3.prototype.onConnection = function (_handle) { console.error('PROBE js: BBProbeScreen3 connected'); };
BBProbeScreen3.prototype.onDisconnection = function () {};
registerScreen('BBProbeScreen3', new BBProbeScreen3());

BbProbe.noop = function () {};
BbProbe.run = function ()
{
	var h = BbProbe.handle;
	var results = {};
	var names = ['plainRet', 'asyncRet', 'connectRet', 'newScreenRet'];
	var next = function (i)
	{
		if (i >= names.length)
		{
			setTimeout(function ()
			{
				BbProbe.send('callbacks', JSON.stringify(results));
				BbProbe.done();
			}, 2000);
			return;
		}
		var name = names[i];
		results[name] = 'no callback';
		setTimeout(function ()
		{
			SQ.call(h, name, null, function (r) { results[name] = (typeof r) + ' ' + JSON.stringify(r); });
			setTimeout(function () { next(i + 1); }, 1000);
		}, 200);
	};
	next(0);
};
