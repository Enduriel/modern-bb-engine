//! Floats become text as in the original engine, whose Squirrel 3.0 formats them with
//! the MSVC UCRT's printf: exact decimal digits, exact ties rounded away from zero,
//! "-nan(ind)" for the x86 default NaN. Ours does it with its own formatter
//! (squirrel-sys `sq_formatfloat`), so no platform C library is involved. Every
//! expected value here was read from the original game.

use super::SQVM;

/// Run `checks` in f32 math mode (literals and arithmetic as the original's 32-bit
/// floats, as game scripts run), after a prelude that defines `eq(label, f, want)`
/// (records a mismatch) and the special values `inf`, `ninf`, `nan` (the x86 default
/// NaN that inf - inf makes, negative), `pnan` (its negation) and `nzero`. Panics
/// listing every mismatch.
fn run_f32(checks: &str) {
	let vm = SQVM::new(1024);
	vm.run_script(
		r#"
		::fails <- [];
		::eq <- function(label, f, want) {
			local got;
			try { got = f(); } catch (e) { got = "ERROR: " + e; }
			if (got != want) ::fails.push(label + ": got \"" + got + "\", want \"" + want + "\"");
		};
		"#,
		"prelude",
	)
	.expect("prelude");
	let source = format!(
		r#"
		local big = 1e38;
		local inf = big * big;
		local ninf = -inf;
		local nan = inf - inf;
		local pnan = -nan;
		local nzero = -0.0;
		local nzero2 = 0.0 * -1.0;
		{checks}
		if (::fails.len() > 0) {{
			local all = "";
			foreach (f in ::fails) all += "\n" + f;
			throw ::fails.len() + " mismatches:" + all;
		}}
		"#
	);
	let closure = vm.compile(&source, "checks").expect("compile");
	assert!(closure.set_f32_math(&vm, true));
	if let Err(e) = closure.call(&vm, ()) {
		panic!("{e}");
	}
}

#[test]
fn format_float_conversions_match_the_original() {
	// Left out, as they differ by storage width only: an integer given to %f/%e/%g
	// becomes a 32-bit float in the original (16777217 prints 16777216.0), a 64-bit
	// one here.
	run_f32(
		r#"
		::eq("%.0f 0.5", @() format("%.0f", 0.5), "1");
		::eq("%.0f 1.5", @() format("%.0f", 1.5), "2");
		::eq("%.0f 2.5", @() format("%.0f", 2.5), "3");
		::eq("%.0f 3.5", @() format("%.0f", 3.5), "4");
		::eq("%.0f 4.5", @() format("%.0f", 4.5), "5");
		::eq("%.0f -0.5", @() format("%.0f", -0.5), "-1");
		::eq("%.0f -1.5", @() format("%.0f", -1.5), "-2");
		::eq("%.0f -2.5", @() format("%.0f", -2.5), "-3");
		::eq("%.0f 0.49999997", @() format("%.0f", 0.49999997), "0");
		::eq("%.0f 0.50000006", @() format("%.0f", 0.50000006), "1");
		::eq("%.0f -0.4", @() format("%.0f", -0.4), "-0");
		::eq("%.0f 16777216", @() format("%.0f", 16777216.0), "16777216");
		::eq("%.0f 8388607.5", @() format("%.0f", 8388607.5), "8388608");
		::eq("%.0f 1e20", @() format("%.0f", 1e20), "100000002004087734272");
		::eq("%.1f 0.25", @() format("%.1f", 0.25), "0.3");
		::eq("%.1f 0.75", @() format("%.1f", 0.75), "0.8");
		::eq("%.1f 2.25", @() format("%.1f", 2.25), "2.3");
		::eq("%.1f -2.25", @() format("%.1f", -2.25), "-2.3");
		::eq("%.1f 0.35", @() format("%.1f", 0.35), "0.3");
		::eq("%.1f 0.45", @() format("%.1f", 0.45), "0.4");
		::eq("%.1f 0.05", @() format("%.1f", 0.05), "0.1");
		::eq("%.1f 0.15", @() format("%.1f", 0.15), "0.2");
		::eq("%.1f 0.95", @() format("%.1f", 0.95), "0.9");
		::eq("%.1f 9.95", @() format("%.1f", 9.95), "9.9");
		::eq("%.1f 99.95", @() format("%.1f", 99.95), "99.9");
		::eq("%.2f 0.125", @() format("%.2f", 0.125), "0.13");
		::eq("%.2f 0.375", @() format("%.2f", 0.375), "0.38");
		::eq("%.2f -0.125", @() format("%.2f", -0.125), "-0.13");
		::eq("%.2f 10.125", @() format("%.2f", 10.125), "10.13");
		::eq("%.2f 1.005", @() format("%.2f", 1.005), "1.00");
		::eq("%.2f 2.675", @() format("%.2f", 2.675), "2.67");
		::eq("%.2f 0.615", @() format("%.2f", 0.615), "0.62");
		::eq("%.2f 0.995", @() format("%.2f", 0.995), "1.00");
		::eq("%.2f 99.995", @() format("%.2f", 99.995), "100.00");
		::eq("%.2f 0.005", @() format("%.2f", 0.005), "0.00");
		::eq("%.2f 0.015", @() format("%.2f", 0.015), "0.01");
		::eq("%.3f 1.0005", @() format("%.3f", 1.0005), "1.000");
		::eq("%.3f 0.0625", @() format("%.3f", 0.0625), "0.063");
		::eq("%.3f 1.0625", @() format("%.3f", 1.0625), "1.063");
		::eq("%.3f -0.0625", @() format("%.3f", -0.0625), "-0.063");
		::eq("%.4f 0.03125", @() format("%.4f", 0.03125), "0.0313");
		::eq("%.4f 0.09375", @() format("%.4f", 0.09375), "0.0938");
		::eq("%f 1/3", @() format("%f", 1.0 / 3.0), "0.333333");
		::eq("%f 0.1", @() format("%f", 0.1), "0.100000");
		::eq("%f 100", @() format("%f", 100.0), "100.000000");
		::eq("%f 1e-7", @() format("%f", 1e-7), "0.000000");
		::eq("%f 123456.789", @() format("%f", 123456.789), "123456.789063");
		::eq("%.10f 0.1", @() format("%.10f", 0.1), "0.1000000015");
		::eq("%.20f 0.1", @() format("%.20f", 0.1), "0.10000000149011611938");
		::eq("%.30f 0.1", @() format("%.30f", 0.1), "0.100000001490116119384765625000");
		::eq("%.60f 0.1", @() format("%.60f", 0.1), "0.100000001490116119384765625000000000000000000000000000000000");
		::eq("%.50f 1e-30", @() format("%.50f", 1e-30), "0.00000000000000000000000000000100000000317107685097");
		::eq("%f 3.4028235e38", @() format("%f", 3.4028235e38), "340282346638528859811704183484516925440.000000");
		::eq("%.0f 3.4028235e38", @() format("%.0f", 3.4028235e38), "340282346638528859811704183484516925440");
		::eq("%f 1e10", @() format("%f", 1e10), "10000000000.000000");
		::eq("%.30f 0.0000002384185791015625", @() format("%.30f", 0.0000002384185791015625), "0.000000238418579101562500000000");
		::eq("%.60f 1.4e-45", @() format("%.60f", 1.4e-45), "0.000000000000000000000000000000000000000000001401298464324817");
		::eq("%f 1.4e-45", @() format("%f", 1.4e-45), "0.000000");
		::eq("%e 0", @() format("%e", 0.0), "0.000000e+00");
		::eq("%e 1", @() format("%e", 1.0), "1.000000e+00");
		::eq("%e 1234.5", @() format("%e", 1234.5), "1.234500e+03");
		::eq("%.0e 2.5", @() format("%.0e", 2.5), "3e+00");
		::eq("%.0e 3.5", @() format("%.0e", 3.5), "4e+00");
		::eq("%.0e 25", @() format("%.0e", 25.0), "3e+01");
		::eq("%.2e 1.125", @() format("%.2e", 1.125), "1.13e+00");
		::eq("%.2e 1.375", @() format("%.2e", 1.375), "1.38e+00");
		::eq("%.2e -1.125", @() format("%.2e", -1.125), "-1.13e+00");
		::eq("%E 0.1", @() format("%E", 0.1), "1.000000E-01");
		::eq("%.3e 1e38", @() format("%.3e", 1e38), "1.000e+38");
		::eq("%.3e 1e-38", @() format("%.3e", 1e-38), "1.000e-38");
		::eq("%e 1.4e-45", @() format("%e", 1.4e-45), "1.401298e-45");
		::eq("%e 1e-40", @() format("%e", 1e-40), "9.999946e-41");
		::eq("%.2e 9.9999999", @() format("%.2e", 9.9999999), "1.00e+01");
		::eq("%.2e 9.995", @() format("%.2e", 9.995), "9.99e+00");
		::eq("%.20e 0.1", @() format("%.20e", 0.1), "1.00000001490116119385e-01");
		::eq("%.60e 0.1", @() format("%.60e", 0.1), "1.000000014901161193847656250000000000000000000000000000000000e-01");
		::eq("%.0e 123", @() format("%.0e", 123.0), "1e+02");
		::eq("%e 1e10", @() format("%e", 1e10), "1.000000e+10");
		::eq("%.4e 99999.5", @() format("%.4e", 99999.5), "1.0000e+05");
		::eq("%g 0", @() format("%g", 0.0), "0");
		::eq("%g 1", @() format("%g", 1.0), "1");
		::eq("%g 0.5", @() format("%g", 0.5), "0.5");
		::eq("%g 100000", @() format("%g", 100000.0), "100000");
		::eq("%g 1000000", @() format("%g", 1000000.0), "1e+06");
		::eq("%g 1234567", @() format("%g", 1234567.0), "1.23457e+06");
		::eq("%g 1234565", @() format("%g", 1234565.0), "1.23457e+06");
		::eq("%g 1234575", @() format("%g", 1234575.0), "1.23458e+06");
		::eq("%g 999999.5", @() format("%g", 999999.5), "1e+06");
		::eq("%g 0.0001", @() format("%g", 0.0001), "0.0001");
		::eq("%g 0.00001", @() format("%g", 0.00001), "1e-05");
		::eq("%g 123456", @() format("%g", 123456.0), "123456");
		::eq("%.1g 2.5", @() format("%.1g", 2.5), "3");
		::eq("%.0g 2.5", @() format("%.0g", 2.5), "3");
		::eq("%.1g 3.5", @() format("%.1g", 3.5), "4");
		::eq("%.2g 0.125", @() format("%.2g", 0.125), "0.13");
		::eq("%#g 1", @() format("%#g", 1.0), "1.00000");
		::eq("%#g 100000", @() format("%#g", 100000.0), "100000.");
		::eq("%G 1e-10", @() format("%G", 1e-10), "1E-10");
		::eq("%g 1e20", @() format("%g", 1e20), "1e+20");
		::eq("%.10g 0.1", @() format("%.10g", 0.1), "0.1000000015");
		::eq("%.20g 0.1", @() format("%.20g", 0.1), "0.10000000149011611938");
		::eq("%g 3.4028235e38", @() format("%g", 3.4028235e38), "3.40282e+38");
		::eq("%#.3g 1", @() format("%#.3g", 1.0), "1.00");
		::eq("%g 1e-5", @() format("%g", 1e-5), "1e-05");
		::eq("%g 0.0001234565", @() format("%g", 0.00012345650), "0.000123456");
		::eq("%g 1/3", @() format("%g", 1.0 / 3.0), "0.333333");
		::eq("%.17g 1/3", @() format("%.17g", 1.0 / 3.0), "0.3333333432674408");
		::eq("%.2g 100", @() format("%.2g", 100.0), "1e+02");
		::eq("%.2g 150", @() format("%.2g", 150.0), "1.5e+02");
		::eq("%G 1e15", @() format("%G", 1e15), "1E+15");
		::eq("%.0g 0.5", @() format("%.0g", 0.5), "0.5");
		::eq("%.0g 1.5", @() format("%.0g", 1.5), "2");
		::eq("%.0g 9.5", @() format("%.0g", 9.5), "1e+01");
		::eq("%.1g 95", @() format("%.1g", 95.0), "1e+02");
		::eq("%+f 1.5", @() format("%+f", 1.5), "+1.500000");
		::eq("% f 1.5", @() format("% f", 1.5), " 1.500000");
		::eq("% f -1.5", @() format("% f", -1.5), "-1.500000");
		::eq("[%-10.2f] 1.5", @() format("[%-10.2f]", 1.5), "[1.50      ]");
		::eq("%010.2f -1.5", @() format("%010.2f", -1.5), "-000001.50");
		::eq("%+010.2f 1.5", @() format("%+010.2f", 1.5), "+000001.50");
		::eq("[%-+10.2f] 1.5", @() format("[%-+10.2f]", 1.5), "[+1.50     ]");
		::eq("[%10.3e] 1.5", @() format("[%10.3e]", 1.5), "[ 1.500e+00]");
		::eq("[%-12g] 1.5", @() format("[%-12g]", 1.5), "[1.5         ]");
		::eq("%012g -1e-5", @() format("%012g", -1e-5), "-0000001e-05");
		::eq("% e 1.5", @() format("% e", 1.5), " 1.500000e+00");
		::eq("%#.0f 1", @() format("%#.0f", 1.0), "1.");
		::eq("%#.0e 1", @() format("%#.0e", 1.0), "1.e+00");
		::eq("%#.0f 1.5", @() format("%#.0f", 1.5), "2.");
		::eq("[%5.0f] 1.5", @() format("[%5.0f]", 1.5), "[    2]");
		::eq("[%0-8.1f] 1.5", @() format("[%0-8.1f]", 1.5), "[1.5     ]");
		::eq("[% +f] 1.5", @() format("[% +f]", 1.5), "[+1.500000]");
		::eq("[%+ f] 1.5", @() format("[%+ f]", 1.5), "[+1.500000]");
		::eq("[%15G] 1.5", @() format("[%15G]", 1.5), "[            1.5]");
		::eq("%5.1f%% 2.5", @() format("%5.1f%%", 2.5), "  2.5%");
		::eq("%.f 0.5", @() format("%.f", 0.5), "1");
		::eq("%.e 1.5", @() format("%.e", 1.5), "2e+00");
		::eq("%.g 1.5", @() format("%.g", 1.5), "2");
		::eq("%1.f 0.5", @() format("%1.f", 0.5), "1");
		::eq("%f -0.0", @() format("%f", nzero), "-0.000000");
		::eq("%g -0.0", @() format("%g", nzero), "-0");
		::eq("%e -0.0", @() format("%e", nzero), "-0.000000e+00");
		::eq("%.0f -0.0", @() format("%.0f", nzero), "-0");
		::eq("%f 0*-1", @() format("%f", nzero2), "-0.000000");
		::eq("%+f -0.0", @() format("%+f", nzero), "-0.000000");
		::eq("%+f 0.0", @() format("%+f", 0.0), "+0.000000");
		::eq("% g -0.0", @() format("% g", nzero), "-0");
		::eq("%05.1f -0.0", @() format("%05.1f", nzero), "-00.0");
		::eq("%f inf", @() format("%f", inf), "inf");
		::eq("%f -inf", @() format("%f", ninf), "-inf");
		::eq("%e inf", @() format("%e", inf), "inf");
		::eq("%g inf", @() format("%g", inf), "inf");
		::eq("%E inf", @() format("%E", inf), "INF");
		::eq("%G inf", @() format("%G", inf), "INF");
		::eq("%.0f inf", @() format("%.0f", inf), "inf");
		::eq("%.2f inf", @() format("%.2f", inf), "inf");
		::eq("[%10f] inf", @() format("[%10f]", inf), "[       inf]");
		::eq("[%-10f] inf", @() format("[%-10f]", inf), "[inf       ]");
		::eq("[%010f] inf", @() format("[%010f]", inf), "[0000000inf]");
		::eq("%+f inf", @() format("%+f", inf), "+inf");
		::eq("% f inf", @() format("% f", inf), " inf");
		::eq("%#g inf", @() format("%#g", inf), "inf");
		::eq("%.1e inf", @() format("%.1e", inf), "inf");
		::eq("%g -inf", @() format("%g", ninf), "-inf");
		::eq("%.1f -inf", @() format("%.1f", ninf), "-inf");
		::eq("%f nan", @() format("%f", nan), "-nan(ind)");
		::eq("%e nan", @() format("%e", nan), "-nan(ind)");
		::eq("%g nan", @() format("%g", nan), "-nan(ind)");
		::eq("%G nan", @() format("%G", nan), "-NAN(IND)");
		::eq("%.0f nan", @() format("%.0f", nan), "-nan(ind)");
		::eq("[%12f] nan", @() format("[%12f]", nan), "[   -nan(ind)]");
		::eq("[%-12f] nan", @() format("[%-12f]", nan), "[-nan(ind)   ]");
		::eq("[%012f] nan", @() format("[%012f]", nan), "[-000nan(ind)]");
		::eq("%+f nan", @() format("%+f", nan), "-nan(ind)");
		::eq("%f -nan", @() format("%f", pnan), "nan");
		::eq("%g -nan", @() format("%g", pnan), "nan");
		::eq("%E -nan", @() format("%E", pnan), "NAN");
		::eq("%+f -nan", @() format("%+f", pnan), "+nan");
		::eq("[%12f] -nan", @() format("[%12f]", pnan), "[         nan]");
		::eq("%f int 3", @() format("%f", 3), "3.000000");
		::eq("%e int -5", @() format("%e", -5), "-5.000000e+00");
		::eq("%g int 123456789", @() format("%g", 123456789), "1.23457e+08");
		::eq("multi", @() format("%.0f|%.1f|%5.2f|%e|%g|%d|%s", 2.5, 0.25, 0.125, 2.5, 2.5, 7, "x"),
			"3|0.3| 0.13|2.500000e+00|2.5|7|x");
		"#,
	);
}

#[test]
fn float_tostring_and_concatenation_match_the_original() {
	run_f32(
		r#"
		::eq("tostring 1.0", @() (1.0).tostring(), "1");
		::eq("concat 1.0", @() "" + 1.0, "1");
		::eq("tostring 1.5", @() (1.5).tostring(), "1.5");
		::eq("concat 1.5", @() "" + 1.5, "1.5");
		::eq("tostring 2.5", @() (2.5).tostring(), "2.5");
		::eq("concat 2.5", @() "" + 2.5, "2.5");
		::eq("tostring 0.1", @() (0.1).tostring(), "0.1");
		::eq("concat 0.1", @() "" + 0.1, "0.1");
		::eq("tostring 1/3", @() (1.0 / 3.0).tostring(), "0.333333");
		::eq("concat 1/3", @() "" + 1.0 / 3.0, "0.333333");
		::eq("tostring 100000", @() (100000.0).tostring(), "100000");
		::eq("concat 100000", @() "" + 100000.0, "100000");
		::eq("tostring 1000000", @() (1000000.0).tostring(), "1e+06");
		::eq("concat 1000000", @() "" + 1000000.0, "1e+06");
		::eq("tostring 1234565", @() (1234565.0).tostring(), "1.23457e+06");
		::eq("concat 1234565", @() "" + 1234565.0, "1.23457e+06");
		::eq("tostring 1234575", @() (1234575.0).tostring(), "1.23458e+06");
		::eq("concat 1234575", @() "" + 1234575.0, "1.23458e+06");
		::eq("tostring 2500000", @() (2500000.0).tostring(), "2.5e+06");
		::eq("concat 2500000", @() "" + 2500000.0, "2.5e+06");
		::eq("tostring 0.0001", @() (0.0001).tostring(), "0.0001");
		::eq("concat 0.0001", @() "" + 0.0001, "0.0001");
		::eq("tostring 0.00001", @() (0.00001).tostring(), "1e-05");
		::eq("concat 0.00001", @() "" + 0.00001, "1e-05");
		::eq("tostring 1e-5", @() (1e-5).tostring(), "1e-05");
		::eq("concat 1e-5", @() "" + 1e-5, "1e-05");
		::eq("tostring 1e38", @() (1e38).tostring(), "1e+38");
		::eq("concat 1e38", @() "" + 1e38, "1e+38");
		::eq("tostring 3.4028235e38", @() (3.4028235e38).tostring(), "3.40282e+38");
		::eq("concat 3.4028235e38", @() "" + 3.4028235e38, "3.40282e+38");
		::eq("tostring 1.4e-45", @() (1.4e-45).tostring(), "1.4013e-45");
		::eq("concat 1.4e-45", @() "" + 1.4e-45, "1.4013e-45");
		::eq("tostring -0.0", @() (nzero).tostring(), "-0");
		::eq("concat -0.0", @() "" + nzero, "-0");
		::eq("tostring 0.0", @() (0.0).tostring(), "0");
		::eq("concat 0.0", @() "" + 0.0, "0");
		::eq("tostring inf", @() (inf).tostring(), "inf");
		::eq("concat inf", @() "" + inf, "inf");
		::eq("tostring -inf", @() (ninf).tostring(), "-inf");
		::eq("concat -inf", @() "" + ninf, "-inf");
		::eq("tostring nan", @() (nan).tostring(), "-nan(ind)");
		::eq("concat nan", @() "" + nan, "-nan(ind)");
		::eq("tostring -nan", @() (pnan).tostring(), "nan");
		::eq("concat -nan", @() "" + pnan, "nan");
		::eq("tostring 123456.5", @() (123456.5).tostring(), "123457");
		::eq("concat 123456.5", @() "" + 123456.5, "123457");
		::eq("tostring 999999.5", @() (999999.5).tostring(), "1e+06");
		::eq("concat 999999.5", @() "" + 999999.5, "1e+06");
		::eq("tostring 0.30000001", @() (0.3).tostring(), "0.3");
		::eq("concat 0.30000001", @() "" + 0.3, "0.3");
		::eq("tostring -2.5", @() (-2.5).tostring(), "-2.5");
		::eq("concat -2.5", @() "" + -2.5, "-2.5");
		::eq("tostring 1e15", @() (1e15).tostring(), "1e+15");
		::eq("concat 1e15", @() "" + 1e15, "1e+15");
		::eq("tostring 16777216", @() (16777216.0).tostring(), "1.67772e+07");
		::eq("concat 16777216", @() "" + 16777216.0, "1.67772e+07");
		::eq("tostring 12345.65", @() (12345.65).tostring(), "12345.7");
		::eq("concat 12345.65", @() "" + 12345.65, "12345.7");
		"#,
	);
}

#[test]
fn tofloat_and_float_literals_match_the_original() {
	// Strings whose f64 parse rounds to the same value as the original's f32 one
	// (strtod, then a cast to float). The others differ by storage width only
	// (docs/differences.md, "Floats in `format` and `tofloat`").
	run_f32(
		r#"
		local function tf(s) { return s.tofloat().tostring(); }
		::eq("1.5", @() tf("1.5"), "1.5");
		::eq("1e500", @() tf("1e500"), "inf");
		::eq("hex float", @() tf("0x1.8p1"), "3");
		::eq("leading space", @() tf(" 1.5"), "1.5");
		::eq("leading tab", @() tf("\t2.5"), "2.5");
		::eq("trailing newline", @() tf("2.5\n"), "2.5");
		::eq("trailing junk", @() tf("1.5abc"), "1.5");
		::eq("1.", @() tf("1."), "1");
		::eq(".5", @() tf(".5"), "0.5");
		::eq("-.5e1", @() tf("-.5e1"), "-5");
		::eq("1e5", @() tf("1e5"), "100000");
		::eq("inf", @() tf("inf"), "ERROR: cannot convert the string");
		::eq("1.0e", @() tf("1.0e"), "1");
		::eq("nan.5", @() tf("nan.5"), "nan");
		::eq("-0.0", @() tf("-0.0"), "-0");
		::eq("1,5", @() tf("1,5"), "1");
		::eq("+1.5", @() tf("+1.5"), "1.5");
		::eq("1.5e+3", @() tf("1.5e+3"), "1500");
		::eq("1E3", @() tf("1E3"), "1000");
		::eq("1.#INF", @() tf("1.#INF"), "1");
		::eq("infinity.0", @() tf("infinity.0"), "inf");
		::eq("0x10.0", @() tf("0x10.0"), "16");
		::eq("5e", @() tf("5e"), "5");
		::eq("e5", @() tf("e5"), "ERROR: cannot convert the string");
		::eq(".", @() tf("."), "ERROR: cannot convert the string");
		::eq("-", @() tf("-"), "ERROR: cannot convert the string");
		::eq("0.30000001192092896", @() format("%.30e", "0.30000001192092896".tofloat()),
			"3.000000119209289550781250000000e-01");
		// literals round through double to float, as the original's lexer
		::eq("literal tie", @() format("%.30e", 1.00000005960464477539062500001), "1.000000000000000000000000000000e+00");
		::eq("literal 16777217.0", @() format("%.1f", 16777217.0), "16777216.0");
		::eq("literal 0.1", @() format("%.30e", 0.1), "1.000000014901161193847656250000e-01");
		::eq("tointeger 2.7", @() (2.7).tointeger().tostring(), "2");
		::eq("tointeger -2.7", @() (-2.7).tointeger().tostring(), "-2");
		::eq("d 2.7", @() format("%d", 2.7), "2");
		::eq("d -2.7", @() format("%d", -2.7), "-2");
		"#,
	);
}

/// `sq_formatfloat` against the C library's printf on random doubles (full range,
/// and f32 values, as game scripts hold), random conversions, flags, widths and
/// precisions: the same text everywhere except at exact ties (the cut falls just
/// before the expansion's last digit, a 5), where the C library rounds to even and the
/// original's CRT away from zero. `sq_formatfloat_ieee` (format3_2's) must match the C
/// library everywhere. glibc's: that is the reference (macOS's, for one, keeps a
/// trailing zero `%g` should drop after rounding a tie, as in `%G` of -9049605).
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[test]
fn format_float_agrees_with_libc_apart_from_ties() {
	use std::ffi::{CStr, CString, c_char, c_int};
	unsafe extern "C" {
		fn snprintf(buf: *mut c_char, size: usize, format: *const c_char, ...) -> c_int;
	}
	let libc = |spec: &CStr, v: f64| -> String {
		let mut buf = vec![0u8; 2048];
		let n = unsafe { snprintf(buf.as_mut_ptr() as *mut c_char, buf.len(), spec.as_ptr(), v) };
		buf.truncate(n as usize);
		String::from_utf8(buf).unwrap()
	};
	let ours = |spec: &CStr, v: f64| -> String {
		let n = unsafe { squirrel_sys::sq_formatfloat(std::ptr::null_mut(), 0, spec.as_ptr(), v) };
		let mut buf = vec![0u8; n as usize + 1];
		let m = unsafe { squirrel_sys::sq_formatfloat(buf.as_mut_ptr() as *mut c_char, buf.len() as i64, spec.as_ptr(), v) };
		assert_eq!(n, m);
		buf.truncate(n as usize);
		String::from_utf8(buf).unwrap()
	};
	let ieee = |spec: &CStr, v: f64| -> String {
		let n = unsafe { squirrel_sys::sq_formatfloat_ieee(std::ptr::null_mut(), 0, spec.as_ptr(), v) };
		let mut buf = vec![0u8; n as usize + 1];
		unsafe { squirrel_sys::sq_formatfloat_ieee(buf.as_mut_ptr() as *mut c_char, buf.len() as i64, spec.as_ptr(), v) };
		buf.truncate(n as usize);
		String::from_utf8(buf).unwrap()
	};
	// inf and NaNs: format3_2 writes them as the C library does
	for v in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN, -f64::NAN, f64::from_bits(0x7ff0_0000_0000_0001)] {
		for spec in [c"%f", c"%+e", c"% G", c"%012g", c"%-8.3f", c"%#g"] {
			assert_eq!(ieee(spec, v), libc(spec, v), "ieee {spec:?} of {:#x}", v.to_bits());
		}
	}
	let mut state = 0x9E37_79B9_7F4A_7C15u64;
	let mut next = move || {
		state ^= state << 13;
		state ^= state >> 7;
		state ^= state << 17;
		state
	};
	let mut checked = 0;
	let mut ties = 0;
	for i in 0..200_000 {
		let r = next();
		let v = match i % 3 {
			0 => f64::from_bits(r),
			1 => f32::from_bits(r as u32) as f64,
			// small f32 values with few digits, where ties are common
			_ => (((r >> 8) % 20_000) as f64 - 10_000.0) / [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0][(r & 7) as usize],
		};
		if !v.is_finite() {
			continue;
		}
		let r2 = next();
		let conv = ["f", "e", "E", "g", "G"][(r2 % 5) as usize];
		let flags: String = ["-", "+", " ", "#", "0"].iter().enumerate()
			.filter(|(k, _)| r2 >> (8 + k) & 1 == 1 && r2 >> 16 & 3 == 0).map(|(_, f)| *f).collect();
		let width = if r2 >> 20 & 1 == 1 { ((r2 >> 24) % 30).to_string() } else { String::new() };
		let (prec, p) = match r2 >> 32 & 3 {
			0 => (String::new(), 6),
			1 => (".".to_string(), 0),
			_ => (format!(".{}", (r2 >> 40) % 40), ((r2 >> 40) % 40) as i64),
		};
		let spec = CString::new(format!("%{flags}{width}{prec}{conv}")).unwrap();
		let got = ours(&spec, v);
		let want = libc(&spec, v);
		// format3_2's formatter rounds as C does: the C library's text, ties included
		assert_eq!(ieee(&spec, v), want, "ieee {spec:?} of {v:e} ({:#x})", v.to_bits());
		if got != want {
			// the exact expansion: significant digits and the decimal exponent
			let exact = libc(c"%.1100e", v.abs());
			let (mantissa, exp) = exact.split_once('e').unwrap();
			let digits = mantissa.replace('.', "");
			let last = digits.trim_end_matches('0').len() as i64 - 1;
			let exp: i64 = exp.parse().unwrap();
			let kept = match conv {
				"f" => exp + 1 + p,
				"e" | "E" => p + 1,
				_ => p.max(1),
			};
			assert!(
				last == kept && digits.as_bytes()[last as usize] == b'5',
				"{spec:?} of {v:e} ({:#x}): ours {got:?}, libc {want:?}",
				v.to_bits()
			);
			ties += 1;
		}
		checked += 1;
	}
	assert!(checked > 150_000 && ties > 100, "{checked} checked, {ties} ties");
	// past the stack buffer: the longest expansions, at a precision format can't ask for
	for (spec, v) in [(c"%.1100f", 4.9e-324), (c"%.1100e", 4.9e-324), (c"%.700f", -1.7976931348623157e308), (c"%#.400g", 0.1)] {
		assert_eq!(ours(spec, v), libc(spec, v), "{spec:?} of {v:e}");
	}
}
