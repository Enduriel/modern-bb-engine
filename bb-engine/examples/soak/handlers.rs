//! What a session hears from the engine: uncaught script errors (with their stacks and
//! locals), compile errors, `print` and the game's log lines. Each error is counted by
//! signature and told to the supervisor; the first few of each signature are recorded in
//! full.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::rc::Rc;

use squirrel::{ErrorValue, RuntimeError};
use squirrel_bb::LogLevel;

use crate::record::{Json, clip, signature, tell};

/// Occurrences of a signature recorded in full; the rest are only counted.
const FULL_RECORDS: u32 = 10;
/// The game log's cap per session.
const GAME_LOG_CAP: u64 = 64 << 20;

/// State the handlers share with the session.
pub struct Shared {
	counts: RefCell<BTreeMap<(String, String), u32>>,
	warnings: RefCell<BTreeMap<String, u32>>,
	game_log: RefCell<Option<File>>,
	game_log_bytes: RefCell<u64>,
	/// What the session is doing, attached to each error.
	pub context: RefCell<String>,
}

impl Shared {
	pub fn new(game_log: Option<File>) -> Rc<Self> {
		Rc::new(Shared {
			counts: RefCell::default(),
			warnings: RefCell::default(),
			game_log: RefCell::new(game_log),
			game_log_bytes: RefCell::new(0),
			context: RefCell::new(String::new()),
		})
	}

	/// Count an error of `kind` with signature `sig`, tell the supervisor, and record
	/// `full` (with the context) unless this signature has been recorded enough.
	pub fn error(&self, kind: &str, sig: &str, full: Json) {
		let n = {
			let mut counts = self.counts.borrow_mut();
			let n = counts.entry((kind.to_string(), sig.to_string())).or_default();
			*n += 1;
			*n
		};
		tell("SIG", &format!("{kind}\t{sig}"));
		if n <= FULL_RECORDS {
			full.str("signature", sig).int("occurrence", n as i64).str("context", &self.context.borrow()).emit();
		}
	}

	pub fn error_counts(&self) -> Vec<((String, String), u32)> {
		self.counts.borrow().iter().map(|(k, v)| (k.clone(), *v)).collect()
	}

	pub fn warning_counts(&self) -> Vec<(String, u32)> {
		let mut w: Vec<(String, u32)> = self.warnings.borrow().iter().map(|(k, v)| (k.clone(), *v)).collect();
		w.sort_by(|a, b| b.1.cmp(&a.1));
		w
	}

	pub fn game_log(&self, line: &str) {
		let mut bytes = self.game_log_bytes.borrow_mut();
		if *bytes > GAME_LOG_CAP {
			return;
		}
		if let Some(f) = self.game_log.borrow_mut().as_mut() {
			let _ = writeln!(f, "{line}");
			*bytes += line.len() as u64 + 1;
			if *bytes > GAME_LOG_CAP {
				let _ = writeln!(f, "(the log reached its cap: nothing more is written)");
			}
		}
	}
}

fn value(v: &ErrorValue) -> String {
	match v {
		ErrorValue::Null => "null".into(),
		ErrorValue::Integer(i) => i.to_string(),
		ErrorValue::Float(f) => f.to_string(),
		ErrorValue::Bool(b) => b.to_string(),
		ErrorValue::String(s) => format!("\"{}\"", clip(&String::from_utf8_lossy(s), 80)),
		other => format!("({other:?})").to_lowercase(),
	}
}

/// Route the engine's script errors, compile errors, prints and log lines to `shared`.
pub fn install(engine: &mut bb_engine::harness::GameEngine, shared: &Rc<Shared>) {
	let s = shared.clone();
	engine.vm_mut().set_runtime_error_handler(move |error: &RuntimeError| {
		eprint!("{}", squirrel_bb::terminal_text(error));
		let message = match &error.error {
			ErrorValue::String(b) => String::from_utf8_lossy(b).into_owned(),
			other => format!("(a thrown {})", value(other)),
		};
		let stack: Vec<String> = error
			.frames
			.iter()
			.map(|f| {
				format!(
					"{} -> {} : {}",
					f.function.as_deref().unwrap_or("<anonymous>"),
					f.source.as_deref().unwrap_or("?"),
					f.line
				)
			})
			.collect();
		let locals: Vec<String> = error
			.frames
			.iter()
			.filter(|f| !f.locals.is_empty())
			.take(3)
			.map(|f| {
				let ls: Vec<String> = f.locals.iter().take(12).map(|l| format!("{} = {}", l.name, value(&l.value))).collect();
				format!("{}: {}", f.function.as_deref().unwrap_or("<anonymous>"), ls.join(", "))
			})
			.collect();
		let ours = error.frames.iter().any(|f| f.source.as_deref().is_some_and(|src| src.starts_with("soak")));
		let place = error
			.frames
			.iter()
			.find(|f| f.source.as_deref().is_some_and(|src| src != "NATIVE"))
			.map(|f| format!("{} ({})", f.function.as_deref().unwrap_or("<anonymous>"), f.source.as_deref().unwrap_or("?")))
			.unwrap_or_default();
		let kind = if ours { "soak_script_error" } else { "script_error" };
		let sig = signature(&format!("{message} @ {place}"));
		s.game_log(&format!("[Script Error] {message} @ {place}"));
		s.error(kind, &sig, Json::new(kind).str("message", &message).strs("stack", &stack).strs("locals", &locals));
	});
	let s = shared.clone();
	engine.vm_mut().set_compile_error_handler(move |e| {
		eprintln!("compile error: {} ({}:{}:{})", e.message, e.source, e.line, e.column);
		let sig = signature(&format!("{} @ {}", e.message, e.source));
		s.error(
			"compile_error",
			&sig,
			Json::new("compile_error").str("message", &e.message).str("source", &e.source).int("line", e.line).int("column", e.column),
		);
	});
	let s = shared.clone();
	engine.vm_mut().set_print_handler(move |text| s.game_log(&format!("[print] {}", text.trim_end())));
	let s = shared.clone();
	engine.set_log_handler(move |level, message| {
		s.game_log(&format!("[{level:?}] {message}"));
		match level {
			LogLevel::Error => {
				// Up to the first sentence's end: what follows is often a name.
				let head = message.split(". ").next().unwrap_or(message);
				let sig = signature(head);
				s.error("log_error", &sig, Json::new("log_error").str("message", &clip(message, 4000)));
			}
			LogLevel::Warning => *s.warnings.borrow_mut().entry(signature(message)).or_default() += 1,
			_ => {}
		}
	});
}
