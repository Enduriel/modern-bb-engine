//! The host's Rust target, the default target to package for (`BB_PACKAGE_HOST`).

fn main() {
	println!("cargo::rustc-env=BB_PACKAGE_HOST={}", std::env::var("TARGET").unwrap_or_default());
}
