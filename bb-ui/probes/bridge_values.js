// UI bridge ground truth: value conversions (JS side). ES5 only. See README.md.
var BBProbe = { echoes: [] };
function jsLog(msg) { console.error('PROBE js: ' + msg); }

BbProbeScreen.prototype.echo = function (_args)
{
	var keys = null;
	if (_args !== null && typeof _args === 'object') keys = Object.keys(_args);
	BBProbe.echoes.push({ type: typeof _args, json: JSON.stringify(_args), keys: keys, isArray: Array.isArray(_args) });
};
BbProbeScreen.prototype.retVal = function (_v) { return { x: _v }; };
BbProbeScreen.prototype.jsThrows = function (_v) { jsLog('jsThrows runs'); throw new Error('js boom'); };

BbProbe.run = function ()
{
	var h = BbProbe.handle;
	var root = Screens.RootScreen;
	BbProbe.send('root', JSON.stringify({
		root: [typeof root.mSQHandle, root.mSQHandle],
		fps: [typeof root.mFPSModule.mSQHandle, root.mFPSModule.mSQHandle],
		version: [typeof root.mVersionModule.mSQHandle, root.mVersionModule.mSQHandle],
		versionHtml: root.mVersionModule.mVersion ? root.mVersionModule.mVersion.html() : null,
		rootClasses: $('.root-screen').attr('class'),
		rootChildren: $('.root-screen').children().map(function () { return this.className; }).get()
	}));

	SQ.call(h, 'types', [1, 2.9, -2.9, 1e10, -1e10, 2147483647.5, null, true, 's', [1, [2.5, null]], { a: 1.5, b: [true, null], c: { d: 'e' } }, [1, , undefined, 3], { u: undefined, f: function () {} }, NaN, Infinity]);
	SQ.call(h, 'types', { '1': 'numeric string key', x: 0.5 });
	SQ.call(h, 'types', [function () {}, { f: [function () {}], o: new Date(0) }, /re/]);
	SQ.call(h, 'pushEchoes');

	setTimeout(function ()
	{
		BbProbe.send('echoes', JSON.stringify(BBProbe.echoes));
		BbProbe.done();
	}, 1500);
};
