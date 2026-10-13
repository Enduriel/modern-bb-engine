//! With the `cef` feature: point the binaries at the CEF distribution the
//! `cef-dll-sys` build downloaded (an rpath for `libcef.so`, and `BB_CEF_DIR` for its
//! resources), so examples and tests run straight from `target/`.

fn main() {
	println!("cargo::rerun-if-env-changed=DEP_CEF_DLL_WRAPPER_CEF_DIR");
	let Ok(dir) = std::env::var("DEP_CEF_DLL_WRAPPER_CEF_DIR") else {
		return;
	};
	println!("cargo::rustc-env=BB_CEF_DIR={dir}");
	if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
		println!("cargo::rustc-link-arg=-Wl,-rpath,{dir}");
	}
}
