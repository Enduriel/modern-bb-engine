//! With the `ui` feature: point `bb` at the CEF distribution the `cef-dll-sys` build
//! downloaded (an rpath for `libcef.so`), so it runs straight from `target/`.

fn main() {
	println!("cargo::rerun-if-env-changed=DEP_CEF_DLL_WRAPPER_CEF_DIR");
	let Ok(dir) = std::env::var("DEP_CEF_DLL_WRAPPER_CEF_DIR") else {
		return;
	};
	if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
		println!("cargo::rustc-link-arg=-Wl,-rpath,{dir}");
	}
}
