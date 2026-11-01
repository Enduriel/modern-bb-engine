//! A zip entry's MS-DOS modification time as Unix seconds, as the original's file layer
//! (PhysFS) reads it: the fields as a local date and time, normalised the way the C
//! library's `mktime` does (month 0 is December of the year before, day 0 the last day
//! of the month before, second 60 the next minute).

use jiff::Span;
use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;

/// `dos` (date in the high 16 bits, time in the low) as Unix seconds in the time zone
/// `tz`, or `None` if it makes no date jiff can hold.
pub(super) fn to_unix_in(dos: u32, tz: &TimeZone) -> Option<i64> {
	let (date, time) = (i64::from(dos >> 16), i64::from(dos & 0xffff));
	// Months first (a month's length depends on which it is), then days and seconds.
	let months = (1980 + (date >> 9 & 0x7f)) * 12 + (date >> 5 & 0x0f) - 1;
	let first = Date::new(months.div_euclid(12) as i16, (months.rem_euclid(12) + 1) as i8, 1).ok()?;
	let seconds = (time >> 11 & 0x1f) * 3600 + (time >> 5 & 0x3f) * 60 + (time << 1 & 0x3e);
	let civil = DateTime::from(first).checked_add(Span::new().days((date & 0x1f) - 1).seconds(seconds)).ok()?;
	Some(civil.to_zoned(tz.clone()).ok()?.timestamp().as_second())
}

/// [`to_unix_in`] the system's time zone, as the original reads it.
pub(super) fn to_unix(dos: u32) -> Option<i64> {
	to_unix_in(dos, &TimeZone::system())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn dos(year: u32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> u32 {
		(year - 1980) << 25 | month << 21 | day << 16 | hour << 11 | minute << 5 | second / 2
	}

	#[test]
	fn reads_the_fields_as_a_date_in_the_time_zone() {
		let utc = TimeZone::UTC;
		// 2025-04-17 10:35:40, the time on vanilla's scripts.
		assert_eq!(to_unix_in(dos(2025, 4, 17, 10, 35, 40), &utc), Some(1_744_886_140));
		let plus_two = TimeZone::fixed(jiff::tz::offset(2));
		assert_eq!(to_unix_in(dos(2025, 4, 17, 10, 35, 40), &plus_two), Some(1_744_886_140 - 7200));
	}

	#[test]
	fn normalises_out_of_range_fields_as_mktime_does() {
		let utc = TimeZone::UTC;
		let at = |y, mo, d, h, mi, s| to_unix_in(dos(y, mo, d, h, mi, s), &utc);
		// Month 0 is December of the year before; day 0 the last day of the month before.
		assert_eq!(at(2020, 0, 1, 0, 0, 0), at(2019, 12, 1, 0, 0, 0));
		assert_eq!(at(2020, 3, 0, 0, 0, 0), at(2020, 2, 29, 0, 0, 0));
		// Month 13 is January of the year after; minute 60 the next hour.
		assert_eq!(at(2020, 13, 1, 0, 0, 0), at(2021, 1, 1, 0, 0, 0));
		assert_eq!(at(2020, 1, 1, 0, 60, 0), at(2020, 1, 1, 1, 0, 0));
		// Second 62 (the field's highest) is two seconds into the next minute.
		assert_eq!(at(2020, 1, 1, 0, 0, 62), at(2020, 1, 1, 0, 1, 2));
	}
}
