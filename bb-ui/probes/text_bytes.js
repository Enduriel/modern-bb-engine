// The page half (see text_bytes.nut): which UTF-16 code units the page gets
// for byte strings from the scripts, and which bytes the scripts get for characters from
// the page. Strings come one at a time, each logged (console.error reaches the log at
// once) before it is asked for: one the original can't convert ends the run. ES5 only;
// the test characters are built from their code points.
BbProbe.run = function ()
{
	var names = (BbProbe.textNames || ['ascii', 'utf8_e_acute', 'utf8_cjk', 'utf8_emoji', 'vanilla_quote',
		'cp1252_euro', 'cp1252_e_acute', 'broken', 'with_nul']);
	var units = function (s)
	{
		var out = [];
		for (var i = 0; i < s.length; i++)
		{
			out.push(s.charCodeAt(i).toString(16));
		}
		return out.join(' ');
	};
	// Likewise one at a time, the lone surrogate last: one of these ends the original with
	// "string too long".
	var back = function ()
	{
		var c = String.fromCharCode;
		var strings = [
			['e_acute', c(0xe9)],
			['cjk', c(0x4e2d)],
			['emoji', c(0xd83d, 0xde00)],
			['euro', c(0x20ac)],
			['c1_control', c(0x80)],
			['with_nul', 'a' + c(0) + 'b'],
			['lone_surrogate', c(0xd83d)]
		];
		for (var i = 0; i < strings.length; i++)
		{
			console.error('PROBE js sending ' + strings[i][0]);
			SQ.call(BbProbe.handle, 'textBytesBack', { name: strings[i][0], value: strings[i][1] });
		}
		setTimeout(function () { BbProbe.done(); }, 2000);
	};
	var next = 0;
	var ask = function ()
	{
		if (next >= names.length)
		{
			back();
			return;
		}
		var name = names[next++];
		console.error('PROBE js asking ' + name);
		SQ.call(BbProbe.handle, 'textBytesGetOne', name, function (s)
		{
			console.error('PROBE js got ' + name + ': ' + (s === null || s === undefined ? String(s) : units(String(s))));
			ask();
		});
	};
	ask();
};
