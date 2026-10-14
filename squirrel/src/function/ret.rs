//! [`IntoSqReturn`]: how a closure's return value is pushed back to Squirrel
//! (or turned into a thrown exception).

use crate::SQVM;

/// A Rust type that can be returned from a native closure back to Squirrel.
pub trait IntoSqReturn {
	/// Push results onto `vm`, returning the number pushed, or an error message
	/// to raise as a Squirrel exception.
	fn push_return(self, vm: &SQVM) -> Result<i64, String>;
}

impl IntoSqReturn for () {
	fn push_return(self, _vm: &SQVM) -> Result<i64, String> {
		Ok(0)
	}
}

impl IntoSqReturn for i64 {
	fn push_return(self, vm: &SQVM) -> Result<i64, String> {
		vm.push_integer(self);
		Ok(1)
	}
}

impl IntoSqReturn for f64 {
	fn push_return(self, vm: &SQVM) -> Result<i64, String> {
		vm.push_float(self);
		Ok(1)
	}
}

impl IntoSqReturn for bool {
	fn push_return(self, vm: &SQVM) -> Result<i64, String> {
		vm.push_bool(self);
		Ok(1)
	}
}

impl IntoSqReturn for String {
	fn push_return(self, vm: &SQVM) -> Result<i64, String> {
		vm.push_string(&self);
		Ok(1)
	}
}

/// A string as its bytes, UTF-8 or not.
impl IntoSqReturn for crate::SqBytes {
	fn push_return(self, vm: &SQVM) -> Result<i64, String> {
		vm.push_bytes(&self.0);
		Ok(1)
	}
}

impl IntoSqReturn for &str {
	fn push_return(self, vm: &SQVM) -> Result<i64, String> {
		vm.push_string(self);
		Ok(1)
	}
}

/// `None` is returned to Squirrel as `null`.
impl<T: IntoSqReturn> IntoSqReturn for Option<T> {
	fn push_return(self, vm: &SQVM) -> Result<i64, String> {
		match self {
			Some(value) => value.push_return(vm),
			None => {
				vm.push_null();
				Ok(1)
			}
		}
	}
}

/// `Err` is raised as a Squirrel exception using its `Display` text.
impl<T: IntoSqReturn, E: std::fmt::Display> IntoSqReturn for Result<T, E> {
	fn push_return(self, vm: &SQVM) -> Result<i64, String> {
		match self {
			Ok(value) => value.push_return(vm),
			Err(error) => Err(error.to_string()),
		}
	}
}
