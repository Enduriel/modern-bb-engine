//! A save's metadata (`PersistenceStorageMetaData`): what the load menu lists, plus what
//! `onBeforeSerialize` records.
//!
//! As in the original (disassembled), the values are one map of strings: `setInt`
//! and `setFloat` format their value (an `ostream <<`), `getInt` and `getFloat` parse the
//! stored string (an `istream >>`: a leading number, 0 if there's none or no such key),
//! `getString` gives "" for a missing key. The map is ordered by bytes, as the
//! original's `std::map<std::string, std::string>` is, which is also the order it's
//! written in.

use std::collections::BTreeMap;

#[derive(Clone, Default, Debug, PartialEq)]
pub(crate) struct Meta {
	pub version: i64,
	/// The label shown in menus (`getName`).
	pub name: String,
	/// The storage name the save is written under (`getFileName`).
	pub file_name: String,
	/// Seconds since the Unix epoch.
	pub created: i64,
	pub modified: i64,
	pub data: BTreeMap<String, String>,
}

impl Meta {
	pub fn get_string(&self, key: &str) -> String {
		self.data.get(key).cloned().unwrap_or_default()
	}

	pub fn set_string(&mut self, key: String, value: String) {
		self.data.insert(key, value);
	}

	pub fn get_int(&self, key: &str) -> i64 {
		self.data.get(key).map_or(0, |v| parse_int(v))
	}

	pub fn set_int(&mut self, key: String, value: i64) {
		self.data.insert(key, value.to_string());
	}

	pub fn get_float(&self, key: &str) -> f64 {
		self.data.get(key).map_or(0.0, |v| parse_float(v))
	}

	pub fn set_float(&mut self, key: String, value: f64) {
		self.data.insert(key, format_float(value as f32));
	}

	pub fn has_data(&self, key: &str) -> bool {
		self.data.contains_key(key)
	}
}

/// The leading part of `s` an `istream` takes as a number: whitespace, then a sign,
/// then what `digit` accepts.
fn number_prefix(s: &str, digit: impl Fn(u8, &[u8]) -> bool) -> &str {
	let s = s.trim_start();
	let bytes = s.as_bytes();
	let mut end = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
	while end < bytes.len() && digit(bytes[end], &bytes[..end]) {
		end += 1;
	}
	&s[..end]
}

/// `istream >> int`: the leading integer, saturated; 0 if there is none.
fn parse_int(s: &str) -> i64 {
	let n = number_prefix(s, |b, _| b.is_ascii_digit());
	match n.parse::<i64>() {
		Ok(v) => v.clamp(i32::MIN.into(), i32::MAX.into()),
		Err(_) if n.trim_start_matches(['+', '-']).is_empty() => 0,
		Err(_) => {
			if n.starts_with('-') {
				i32::MIN.into()
			} else {
				i32::MAX.into()
			}
		}
	}
}

/// `istream >> float`: the leading decimal number; 0 if there is none.
fn parse_float(s: &str) -> f64 {
	let n = number_prefix(s, |b, before| {
		b.is_ascii_digit()
			|| (b == b'.' && !before.contains(&b'.') && !before.iter().any(|&c| c == b'e' || c == b'E'))
			|| ((b == b'e' || b == b'E') && before.iter().any(u8::is_ascii_digit) && !before.iter().any(|&c| c == b'e' || c == b'E'))
			|| ((b == b'+' || b == b'-') && matches!(before.last(), Some(b'e' | b'E')))
	});
	// A trailing exponent marker without digits isn't part of the number.
	let n = n.trim_end_matches(['e', 'E', '+', '-']);
	n.parse::<f32>().map_or(0.0, f64::from)
}

/// `ostream << float` with the default precision: `%g` with 6 significant digits (also
/// how Squirrel's `tostring` prints the original's `f32` floats).
pub(crate) fn format_float(v: f32) -> String {
	if v == 0.0 {
		return if v.is_sign_negative() { "-0" } else { "0" }.to_string();
	}
	if !v.is_finite() {
		return if v.is_nan() { "nan" } else if v > 0.0 { "inf" } else { "-inf" }.to_string();
	}
	let v = f64::from(v);
	// The exponent after rounding to 6 significant digits.
	let sci = format!("{v:.5e}");
	let (mantissa, exp) = sci.split_once('e').expect("{:e} has an exponent");
	let exp: i32 = exp.parse().expect("{:e} exponent");
	let trim = |s: &str| -> String {
		if s.contains('.') {
			s.trim_end_matches('0').trim_end_matches('.').to_string()
		} else {
			s.to_string()
		}
	};
	if (-4..6).contains(&exp) {
		let decimals = (5 - exp).max(0) as usize;
		trim(&format!("{v:.decimals$}"))
	} else {
		let sign = if exp < 0 { '-' } else { '+' };
		format!("{}e{sign}{:02}", trim(mantissa), exp.abs())
	}
}

/// `getCreationDate()`/`getModificationDate()`: the original's `dd.mm.yy HH:MM:SS` (probed `14.10.26 03:47:33`).
/// The original shows local time; without a time-zone source this is UTC.
pub(crate) fn date(secs: i64) -> String {
	let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
	// Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
	let z = days + 719_468;
	let era = z.div_euclid(146_097);
	let doe = z - era * 146_097;
	let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
	let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
	let mp = (5 * doy + 2) / 153;
	let day = doy - (153 * mp + 2) / 5 + 1;
	let month = if mp < 10 { mp + 3 } else { mp - 9 };
	let year = yoe + era * 400 + i64::from(month <= 2);
	format!(
		"{day:02}.{month:02}.{:02} {:02}:{:02}:{:02}",
		year.rem_euclid(100),
		rem / 3600,
		rem % 3600 / 60,
		rem % 60
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn creation_dates_read_like_the_original() {
		// 2026-10-14 01:47:33 UTC.
		assert_eq!(date(1_791_942_453), "14.10.26 01:47:33");
		assert_eq!(date(0), "01.01.70 00:00:00");
		assert_eq!(date(951_782_400), "29.02.00 00:00:00");
	}

	#[test]
	fn values_are_strings_as_in_the_original() {
		let mut m = Meta::default();
		m.set_int("days".into(), 57);
		assert_eq!(m.get_string("days"), "57");
		assert_eq!(m.get_int("days"), 57);
		m.set_string("dlc".into(), " 511 apples".into());
		assert_eq!(m.get_int("dlc"), 511);
		m.set_string("name".into(), "Chosen".into());
		assert_eq!(m.get_int("name"), 0);
		assert_eq!(m.get_int("missing"), 0);
		assert_eq!(m.get_string("missing"), "");
		assert!(m.has_data("name") && !m.has_data("missing"));
		m.set_float("mult".into(), 0.1);
		assert_eq!(m.get_string("mult"), "0.1");
		assert!((m.get_float("mult") - 0.1).abs() < 1e-6);
		assert_eq!(m.get_float("days"), 57.0);
	}

	#[test]
	fn integers_parse_like_an_istream() {
		assert_eq!(parse_int("-12x"), -12);
		assert_eq!(parse_int("+7"), 7);
		assert_eq!(parse_int("-"), 0);
		assert_eq!(parse_int("99999999999"), i32::MAX.into());
		assert_eq!(parse_int("-99999999999"), i32::MIN.into());
	}

	#[test]
	fn floats_format_like_an_ostream() {
		assert_eq!(format_float(1.0), "1");
		assert_eq!(format_float(1.5), "1.5");
		assert_eq!(format_float(123456.0), "123456");
		assert_eq!(format_float(1234567.0), "1.23457e+06");
		assert_eq!(format_float(0.0001), "0.0001");
		assert_eq!(format_float(0.00001), "1e-05");
		assert_eq!(format_float(-2.25), "-2.25");
		assert_eq!(parse_float("1.5e3x"), 1500.0);
		assert_eq!(parse_float("2e"), 2.0);
		assert_eq!(parse_float(".5"), 0.5);
	}
}
