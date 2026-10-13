//! Compiling and running Squirrel scripts, and allocating Rust values as
//! Squirrel userdata (with a release hook that drops them on collection).

use std::ffi::{c_char, c_void};

use squirrel_sys::SQUserPointer;

use super::SQVM;
use crate::{SQ_OK, SqStrongRef};

/// Drops the `T` stored in a userdata block when Squirrel frees it.
unsafe extern "C" fn release_hook<T>(ptr: SQUserPointer, _size: i64) -> i64 {
	unsafe { std::ptr::drop_in_place(ptr as *mut T) };
	0
}

impl SQVM {
	/// Compile and run a Squirrel script. `source_name` is the name the VM reports
	/// for it in errors and debug info (use the script's path so `getstackinfos`
	/// can recover it). Runs with the root table as `this`.
	///
	/// Leak-free: the compiled closure and any return value are popped before
	/// returning, so it is safe to call thousands of times (e.g. loading scripts).
	pub fn run_script(&self, source: &str, source_name: &str) -> Result<(), String> {
		let ptr = self.as_ptr();
		let name = std::ffi::CString::new(source_name)
			.unwrap_or_else(|_| std::ffi::CString::new("script").unwrap());
		unsafe {
			// `sq_compilebuffer` pushes a closure on success, nothing on failure.
			if squirrel_sys::sq_compilebuffer(
				ptr,
				source.as_ptr() as *const c_char,
				source.len() as i64,
				name.as_ptr(),
				1, // starting line number
			) != SQ_OK
			{
				return Err(format!("failed to compile {source_name}"));
			}

			// The compiled main function takes the root table as its `this`.
			squirrel_sys::sq_pushroottable(ptr); // [closure, this]
			if squirrel_sys::sq_call(ptr, 1, 0, 1) == SQ_OK {
				// sq_call popped `this`; the closure stays -> [closure]
				squirrel_sys::sq_pop(ptr, 1);
				Ok(())
			} else {
				// sq_call popped `this`; the closure stays -> [closure]
				squirrel_sys::sq_getlasterror(ptr); // [closure, error]
				let message = self
					.get_str_ref(-1)
					.map(str::to_owned)
					.unwrap_or_else(|_| format!("error running {source_name}"));
				squirrel_sys::sq_pop(ptr, 1); // pop error
				squirrel_sys::sq_reseterror(ptr);
				squirrel_sys::sq_pop(ptr, 1); // pop closure
				Err(message)
			}
		}
	}

	/// Compile `source` into a closure without running it (e.g. to flag it with
	/// [`SqStrongRef::set_f32_math`] first). Run it with [`SqStrongRef::call`].
	pub fn compile(&self, source: &str, source_name: &str) -> Result<SqStrongRef, String> {
		let ptr = self.as_ptr();
		let name = std::ffi::CString::new(source_name)
			.unwrap_or_else(|_| std::ffi::CString::new("script").unwrap());
		unsafe {
			if squirrel_sys::sq_compilebuffer(
				ptr,
				source.as_ptr() as *const c_char,
				source.len() as i64,
				name.as_ptr(),
				1,
			) != SQ_OK
			{
				return Err(format!("failed to compile {source_name}"));
			}
			let closure = SqStrongRef::from_stack(self, -1);
			squirrel_sys::sq_pop(ptr, 1);
			Ok(closure)
		}
	}

	/// Convenience wrapper around [`run_script`](Self::run_script) with a fixed
	/// source name, used only by the in-crate tests.
	#[cfg(test)]
	pub(crate) fn execute_squirrel_script(&mut self, script: &str) -> Result<(), String> {
		self.run_script(script, "script")
	}

	pub(crate) fn squirrel_alloc<'a, T: 'a>(&'a mut self, value: T) -> *mut c_void {
		let size = std::mem::size_of::<T>();
		let ptr = unsafe { squirrel_sys::sq_newuserdata(self.as_ptr(), size as u64) };
		unsafe { std::ptr::write(ptr as *mut T, value) };
		if std::mem::needs_drop::<T>() {
			unsafe { squirrel_sys::sq_setreleasehook(self.as_ptr(), -1, Some(release_hook::<T>)) };
		}
		ptr
	}
}

#[cfg(test)]
mod tests {
	use std::rc::Rc;

	use super::SQVM;

	#[test]
	fn alloc_drops() {
		let mut vm = SQVM::new(1024);
		let val = Rc::new(());

		struct TestAlloc {
			_val: Rc<()>,
		}

		vm.squirrel_alloc(TestAlloc { _val: val.clone() });

		assert_eq!(Rc::strong_count(&val), 2, "Value was not allocated");
		unsafe { squirrel_sys::sq_poptop(vm.as_ptr()) };
		assert_eq!(Rc::strong_count(&val), 1, "Value was not dropped on pop");

		vm.squirrel_alloc(TestAlloc { _val: val.clone() });
		assert_eq!(Rc::strong_count(&val), 2, "Value was not allocated");

		drop(vm);
		assert_eq!(
			Rc::strong_count(&val),
			1,
			"Value was not dropped on VM drop"
		);
	}

	#[test]
	fn run_script_executes_and_does_not_leak_the_stack() {
		let vm = SQVM::new(1024);
		vm.run_script("::counter <- 0;", "init").expect("init");
		// If each run leaked stack slots, ~2000 iterations would overflow the
		// 1024-slot stack and start failing.
		for _ in 0..2000 {
			vm.run_script("::counter = ::counter + 1;", "inc")
				.expect("inc");
		}
		vm.run_script("assert(::counter == 2000);", "check")
			.expect("count held");
	}

	#[test]
	fn print_and_error_handlers_capture_output() {
		use std::cell::RefCell;

		let mut vm = SQVM::new(1024);
		let printed = Rc::new(RefCell::new(String::new()));
		let p = printed.clone();
		vm.set_print_handler(move |s| p.borrow_mut().push_str(s));

		let errored = Rc::new(RefCell::new(String::new()));
		let e = errored.clone();
		vm.set_error_handler(move |s| e.borrow_mut().push_str(s));

		vm.run_script(
			r#"::print("hello"); ::print(" world"); ::error("oops");"#,
			"t",
		)
		.expect("script runs");

		assert_eq!(*printed.borrow(), "hello world");
		assert_eq!(*errored.borrow(), "oops");
	}

	#[test]
	fn print_handler_handles_large_messages_without_truncation() {
		use std::cell::RefCell;

		let mut vm = SQVM::new(1024);
		let out = Rc::new(RefCell::new(String::new()));
		let o = out.clone();
		vm.set_print_handler(move |s| o.borrow_mut().push_str(s));

		// Build a 5000-char string and print it — far past the 256-byte inline
		// buffer, so it exercises the exact-size heap path.
		vm.run_script(
			r#"
			local s = "";
			for (local i = 0; i < 1000; i = i + 1) s += "xyzab";
			::print(s);
			"#,
			"big",
		)
		.expect("large print runs");

		assert_eq!(out.borrow().len(), 5000, "no truncation");
	}

	#[test]
	fn run_script_reports_errors_and_stays_usable() {
		let vm = SQVM::new(1024);
		assert!(
			vm.run_script("this is not valid squirrel", "bad").is_err(),
			"compile error should be reported"
		);
		let err = vm.run_script(r#"throw "kaboom";"#, "boom").unwrap_err();
		assert!(
			err.contains("kaboom"),
			"thrown error text should surface: {err}"
		);
		// The VM is still usable after both error kinds.
		vm.run_script("::ok <- 1;", "after")
			.expect("vm still usable");
	}
}
