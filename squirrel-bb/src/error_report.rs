//! Where an uncaught script error goes ([`report_script_errors`]): its message and call
//! stack, with each frame's locals, to stderr and, as the original logs it, to log.html.

use squirrel::{ErrorValue, RuntimeError, SQVM};

use crate::html_log;

/// Report every uncaught script error: to stderr, and as a `Script Error` row with its
/// stack to log.html (when one is open).
pub fn report_script_errors(vm: &mut SQVM) {
	vm.set_runtime_error_handler(|error| {
		eprint!("{}", terminal_text(error));
		html_log::script_error(error);
	});
}

/// The error as the terminal shows it: the message, then each frame and its locals.
pub fn terminal_text(error: &RuntimeError) -> String {
	let message = match &error.error {
		ErrorValue::String(bytes) => String::from_utf8_lossy(bytes).into_owned(),
		other => format!("(a thrown {})", String::from_utf8_lossy(&html_log::value::local(other))),
	};
	let mut out = format!("Script Error: {message}\n");
	for frame in &error.frames {
		let function = frame.function.as_deref().unwrap_or("<Unknown>");
		match &frame.source {
			Some(source) => out.push_str(&format!("  {function} -> {source} : {}\n", frame.line)),
			None => out.push_str(&format!("  {function}\n")),
		}
		if !frame.locals.is_empty() {
			let locals: Vec<String> = frame
				.locals
				.iter()
				.map(|l| format!("{} = {}", l.name, String::from_utf8_lossy(&html_log::value::local(&l.value))))
				.collect();
			out.push_str(&format!("      {}\n", locals.join(", ")));
		}
	}
	out
}
