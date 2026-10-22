//! With the `ui` feature: point `bb` at the CEF distribution the `cef-dll-sys` build
//! downloaded (an rpath for `libcef.so`), so it runs straight from `target/`. A release
//! package's build (`BB_PACKAGE_RPATH=origin`, set by `bb-package`) looks beside the
//! executable instead (`$ORIGIN`), where the package carries `libcef.so`.
//!
//! Also tells `bb --version` the Rust target it was built for (`BB_TARGET`).

fn main() {
	println!("cargo::rustc-env=BB_TARGET={}", std::env::var("TARGET").unwrap_or_default());
	println!("cargo::rerun-if-env-changed=DEP_CEF_DLL_WRAPPER_CEF_DIR");
	println!("cargo::rerun-if-env-changed=BB_PACKAGE_RPATH");
	let Ok(dir) = std::env::var("DEP_CEF_DLL_WRAPPER_CEF_DIR") else {
		return;
	};
	if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
		if std::env::var("BB_PACKAGE_RPATH").as_deref() == Ok("origin") {
			println!("cargo::rustc-link-arg=-Wl,-rpath,$ORIGIN");
		} else {
			println!("cargo::rustc-link-arg=-Wl,-rpath,{dir}");
		}
	}
}
