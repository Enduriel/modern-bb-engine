// How text bytes cross between the scripts and the UI. Squirrel strings are
// bytes; this offers the JS half (text_bytes.js) strings built byte by byte, and logs the
// bytes of the strings the JS half sends back. Run in menu mode with `--js text_bytes.js`.
local bytes = function ( _codes )
{
	local s = "";

	foreach( c in _codes )
	{
		s += c.tochar();
	}

	return s;
};

// One at a time (the page asks for each by name): a string the bridge can't convert
// ends the original with "A critical error occured: string too long", so the page logs
// each name before asking (see text_bytes.js).
::BbProbe.UI.textBytesGetOne <- function ( _name )
{
	local strings = {
		ascii = "Ab",
		utf8_e_acute = bytes([0xC3, 0xA9]),
		utf8_cjk = bytes([0xE4, 0xB8, 0xAD]),
		utf8_emoji = bytes([0xF0, 0x9F, 0x98, 0x80]),
		vanilla_quote = bytes([0xC3, 0x94, 0xC3, 0x87, 0xC2, 0xA3]),
		cp1252_euro = bytes([0x80]),
		cp1252_e_acute = bytes([0xE9]),
		broken = bytes([0xC3, 0x41]),
		with_nul = bytes([0x61, 0x00, 0x62])
	};
	return strings[_name];
};

::BbProbe.UI.textBytesBack <- function ( _data )
{
	local v = _data.value;
	local hex = "";

	for( local i = 0; i < v.len(); i++ )
	{
		hex += format("%02x ", v[i] & 0xFF);
	}

	::logInfo("PROBE text js->sq " + _data.name + " len=" + v.len() + " bytes=" + hex);
};
::logInfo("PROBE sq: text bytes ready");
