//! How the original writes values in log.html: a float as C++'s `ostream << float`
//! does (`%g`, six significant digits, of the f32), and a local in a script error's
//! stack by its value or its type's name (the original's own names).

use squirrel::ErrorValue;

/// `value` as `std::ostream << (float)value` writes it (MSVC): `%g` with six
/// significant digits, trailing zeros dropped, a two-digit exponent at least.
pub fn float_g6(value: f64) -> String {
	let v = value as f32 as f64;
	if v.is_nan() {
		return "nan".into();
	}
	if v.is_infinite() {
		return if v < 0.0 { "-inf".into() } else { "inf".into() };
	}
	if v == 0.0 {
		return if v.is_sign_negative() { "-0".into() } else { "0".into() };
	}
	// The exponent after rounding to six significant digits decides the style.
	let sci = format!("{v:.5e}");
	let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
	let exp: i32 = exp.parse().unwrap_or(0);
	if exp < -4 || exp >= 6 {
		let mantissa = trim_zeros(mantissa);
		let sign = if exp < 0 { '-' } else { '+' };
		format!("{mantissa}e{sign}{:02}", exp.abs())
	} else {
		let decimals = (5 - exp).max(0) as usize;
		trim_zeros(&format!("{v:.decimals$}")).to_string()
	}
}

/// `text` without the zeros after its decimal point (and the point, if nothing's left).
fn trim_zeros(text: &str) -> &str {
	if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.') } else { text }
}

/// A local of a script error's stack as the original writes it: scalars and strings
/// by value (strings raw, unescaped), anything else by its type's name.
pub fn local(value: &ErrorValue) -> Vec<u8> {
	let name = match value {
		ErrorValue::Null => "Null",
		ErrorValue::Integer(i) => return i.to_string().into_bytes(),
		ErrorValue::Float(f) => return float_g6(*f).into_bytes(),
		ErrorValue::Bool(true) => "True",
		ErrorValue::Bool(false) => "False",
		ErrorValue::String(bytes) => return bytes.clone(),
		ErrorValue::Table => "Table",
		ErrorValue::Array => "Array",
		ErrorValue::UserData => "User Data",
		ErrorValue::Closure => "Function",
		ErrorValue::NativeClosure => "Native Function",
		ErrorValue::Generator => "Generator",
		ErrorValue::UserPointer => "<User Pointer>",
		ErrorValue::Thread => "Thread",
		ErrorValue::Class => "Class",
		ErrorValue::Instance => "Instance",
		ErrorValue::WeakRef => "Weak Reference",
		ErrorValue::FuncProto | ErrorValue::Outer => "<Unknown>",
	};
	name.as_bytes().to_vec()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn floats_as_the_original_writes_them() {
		// The original's own output for these (probe 2026-10-20, a script error's locals).
		for (value, text) in [
			(1.5, "1.5"),
			(0.1, "0.1"),
			(100000.0, "100000"),
			(1234567.0, "1.23457e+06"),
			(0.00001, "1e-05"),
			(-2.25, "-2.25"),
			(3.0, "3"),
			(16777217.0, "1.67772e+07"),
			(1e20, "1e+20"),
			(0.0, "0"),
			(1.25, "1.25"),
		] {
			assert_eq!(float_g6(value), text, "{value}");
		}
		assert_eq!(float_g6(999999.5), "1e+06");
		assert_eq!(float_g6(0.0001), "0.0001");
		assert_eq!(float_g6(123.456789), "123.457");
	}

	#[test]
	fn locals_by_value_or_type_name() {
		assert_eq!(local(&ErrorValue::Null), b"Null");
		assert_eq!(local(&ErrorValue::Integer(-42)), b"-42");
		assert_eq!(local(&ErrorValue::Bool(false)), b"False");
		assert_eq!(local(&ErrorValue::String(b"<b>x</b> & y\nz".to_vec())), b"<b>x</b> & y\nz");
		assert_eq!(local(&ErrorValue::Closure), b"Function");
		assert_eq!(local(&ErrorValue::WeakRef), b"Weak Reference");
	}
}
