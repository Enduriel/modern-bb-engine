//! Compile-check Squirrel source files without running them, e.g. a script before
//! running it in the original game. Prints each file's compile error, if any; exits 1 if
//! one failed.
//!
//! Run: `cargo run --release -p squirrel --example sq_check -- FILE.nut...`

use squirrel::SQVM;

fn main() {
	let vm = SQVM::new(1024);
	let mut failed = false;
	for path in std::env::args().skip(1) {
		let source = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
		match vm.compile(&source, &path) {
			Ok(_) => println!("{path}: ok"),
			Err(e) => {
				println!("{path}: {e}");
				failed = true;
			}
		}
	}
	std::process::exit(failed as i32);
}
