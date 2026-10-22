//! With the `ui` feature: point `bb` at the CEF distribution the `cef-dll-sys` build
//! downloaded (an rpath for `libcef.so`), so it runs straight from `target/`. A release
//! package's build (`BB_PACKAGE_RPATH=origin`, set by `bb-package`) looks beside the
//! executable instead (`$ORIGIN`), where the package carries `libcef.so`.
//!
//! Also tells `bb --version` the Rust target it was built for (`BB_TARGET`), and on
//! Windows (MSVC) links the game's icon into `bb.exe` as its resource (`src/icon/res.rs`).

#[path = "src/icon/ico.rs"]
mod ico;
#[path = "src/icon/res.rs"]
#[allow(dead_code)]
mod res;

fn main() {
	println!("cargo::rustc-env=BB_TARGET={}", std::env::var("TARGET").unwrap_or_default());
	icon_resource();
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

/// Windows: `bb.exe`'s icon, as a `.res` file the linker takes in (the binaries only).
fn icon_resource() {
	println!("cargo::rerun-if-changed=assets/bb.ico");
	let target = |key: &str| std::env::var(key).unwrap_or_default();
	if target("CARGO_CFG_TARGET_OS") != "windows" || target("CARGO_CFG_TARGET_ENV") != "msvc" {
		return;
	}
	let file = std::fs::read("assets/bb.ico").expect("assets/bb.ico");
	let images = ico::images(&file).expect("assets/bb.ico");
	let path = std::path::Path::new(&target("OUT_DIR")).join("bb-icon.res");
	std::fs::write(&path, res::icon_res(&images)).expect("the icon's .res file");
	println!("cargo::rustc-link-arg-bins={}", path.display());
}
