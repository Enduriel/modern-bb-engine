//! The profiler's bookkeeping: the call tree, the open calls and the clock.
//!
//! Times are on a virtual clock that stops while the profiler handles an event
//! ([`Recorder::event`]), so its own work isn't charged to any function.

use super::hash::PtrMap;
use super::{CallNode, FunctionKind, Profile, ProfiledFunction};

/// A function's identity while profiling: for a script function, the addresses of its
/// (interned) source and name strings and its first line; for a native, its label's
/// address and length.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Key {
	pub a: usize,
	pub b: usize,
	pub line: i64,
}

/// What a script function's return event names: its source and name.
pub(super) type ReturnKey = (usize, usize);

/// A function's description, made the first time it's seen.
pub(super) struct FunctionInfo {
	pub kind: FunctionKind,
	pub name: Option<String>,
	pub source: Option<String>,
	pub line: i64,
}

const ROOT: u32 = u32::MAX;

/// How far down the open calls a return looks for the call it closes.
const RETURN_SEARCH: usize = 16;

struct Node {
	function: u32,
	parent: u32,
	calls: u64,
	self_ns: u64,
	total_ns: u64,
}

struct Open {
	node: u32,
	start: u64,
	/// `None` for a native call (closed by its own exit, never by a return event).
	returns: Option<ReturnKey>,
}

#[derive(Default)]
pub(super) struct Recorder {
	functions: Vec<FunctionInfo>,
	function_index: PtrMap<Key, u32>,
	nodes: Vec<Node>,
	children: PtrMap<(u32, Key), u32>,
	open: Vec<Open>,
	/// The virtual time of the last event's end.
	last: u64,
	/// The profiler's own time so far: real time minus this is virtual time.
	overhead: u64,
	scope_start: u64,
	wall_ns: u64,
	outside_ns: u64,
	events: u64,
	unmatched: u64,
	runs: u64,
}

impl Recorder {
	/// The virtual time for real time `real`.
	pub fn virtual_time(&self, real: u64) -> u64 {
		real - self.overhead
	}

	/// The profiler spent `real_spent` on an event.
	pub fn add_overhead(&mut self, real_spent: u64) {
		self.overhead += real_spent;
	}

	pub fn begin(&mut self, real: u64) {
		self.runs += 1;
		let now = self.virtual_time(real);
		self.last = now;
		self.scope_start = now;
	}

	pub fn end(&mut self, real: u64) {
		let now = self.virtual_time(real);
		self.charge(now);
		while !self.open.is_empty() {
			self.pop(now);
		}
		self.wall_ns += now - self.scope_start;
	}

	/// Charge the time since the last event to the innermost open call.
	fn charge(&mut self, now: u64) {
		let spent = now.saturating_sub(self.last);
		match self.open.last() {
			Some(open) => self.nodes[open.node as usize].self_ns += spent,
			None => self.outside_ns += spent,
		}
		self.last = now;
	}

	fn pop(&mut self, now: u64) {
		if let Some(open) = self.open.pop() {
			let node = &mut self.nodes[open.node as usize];
			node.total_ns += now.saturating_sub(open.start);
		}
	}

	/// A call of `key` starts; `info` describes it if it's new.
	pub fn enter(&mut self, now: u64, key: Key, returns: Option<ReturnKey>, info: impl FnOnce() -> FunctionInfo) -> usize {
		self.events += 1;
		self.charge(now);
		let parent = self.open.last().map_or(ROOT, |o| o.node);
		let node = match self.children.get(&(parent, key)) {
			Some(&node) => node,
			None => {
				let function = match self.function_index.get(&key) {
					Some(&f) => f,
					None => {
						let f = self.functions.len() as u32;
						self.functions.push(info());
						self.function_index.insert(key, f);
						f
					}
				};
				let node = self.nodes.len() as u32;
				self.nodes.push(Node { function, parent, calls: 0, self_ns: 0, total_ns: 0 });
				self.children.insert((parent, key), node);
				node
			}
		};
		self.nodes[node as usize].calls += 1;
		let depth = self.open.len();
		self.open.push(Open { node, start: now, returns });
		depth
	}

	/// A script function named by `key` returns.
	pub fn script_return(&mut self, now: u64, key: ReturnKey) {
		self.events += 1;
		self.charge(now);
		let len = self.open.len();
		let found = self.open[len.saturating_sub(RETURN_SEARCH)..]
			.iter()
			.rev()
			.take_while(|o| o.returns.is_some())
			.position(|o| o.returns == Some(key));
		match found {
			Some(0) => self.pop(now),
			Some(above) => {
				// Calls left open above it (a generator that yielded) close with it.
				self.unmatched += 1;
				for _ in 0..=above {
					self.pop(now);
				}
			}
			// A tail call's caller: its frame became the callee's, so it returns under
			// the callee's name (once per call the frame made). Closes the innermost
			// script call.
			None if self.open.last().is_some_and(|o| o.returns.is_some()) => self.pop(now),
			// A return without its call (the profiler attached inside it).
			None => self.unmatched += 1,
		}
	}

	/// The native call opened at `depth` returns (and anything left open above it).
	pub fn native_return(&mut self, now: u64, depth: usize) {
		self.events += 1;
		self.charge(now);
		if self.open.len() > depth + 1 {
			self.unmatched += 1;
		}
		while self.open.len() > depth {
			self.pop(now);
		}
	}

	pub fn runs(&self) -> u64 {
		self.runs
	}

	pub fn profile(&self) -> Profile {
		let mut functions: Vec<ProfiledFunction> = self
			.functions
			.iter()
			.map(|f| ProfiledFunction {
				kind: f.kind,
				name: f.name.clone(),
				source: f.source.clone(),
				line: f.line,
				calls: 0,
				self_ns: 0,
				total_ns: 0,
			})
			.collect();
		for node in &self.nodes {
			let f = &mut functions[node.function as usize];
			f.calls += node.calls;
			f.self_ns += node.self_ns;
			// Through recursion, only the outermost call's time counts.
			let mut up = node.parent;
			let mut recursive = false;
			while up != ROOT {
				let parent = &self.nodes[up as usize];
				if parent.function == node.function {
					recursive = true;
					break;
				}
				up = parent.parent;
			}
			if !recursive {
				f.total_ns += node.total_ns;
			}
		}
		Profile {
			functions,
			nodes: self
				.nodes
				.iter()
				.map(|n| CallNode {
					function: n.function as usize,
					parent: (n.parent != ROOT).then_some(n.parent as usize),
					calls: n.calls,
					self_ns: n.self_ns,
					total_ns: n.total_ns,
				})
				.collect(),
			wall_ns: self.wall_ns,
			outside_ns: self.outside_ns,
			overhead_ns: self.overhead,
			events: self.events,
			unmatched: self.unmatched,
			runs: self.runs,
		}
	}
}
