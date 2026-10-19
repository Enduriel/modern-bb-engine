//! Tells the tests which Rust target they were built for (`BB_STEAM_TARGET`).

fn main() {
	println!("cargo::rustc-env=BB_STEAM_TARGET={}", std::env::var("TARGET").unwrap_or_default());
}
