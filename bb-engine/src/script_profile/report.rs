//! The Markdown report of one phase's profile.

use std::collections::HashMap;
use std::fmt::Write;

use squirrel::{FunctionKind, Profile, ProfiledFunction};

use super::owners::{NATIVE, Owners};

/// The call tree shows paths costing at least this much a frame (ms).
const TREE_MIN_MS: f64 = 0.004;
const TREE_MAX_DEPTH: usize = 40;

fn owner<'a>(f: &ProfiledFunction, owners: &'a Owners) -> &'a str {
	match f.kind {
		FunctionKind::Native => NATIVE,
		FunctionKind::Script => owners.of_source(f.source.as_deref()),
	}
}

fn label(f: &ProfiledFunction) -> String {
	let name = f.name.as_deref().unwrap_or("(anonymous)");
	match (f.kind, &f.source) {
		(FunctionKind::Native, _) => shorten_native(name),
		(FunctionKind::Script, Some(source)) => format!("{name} — {source}:{}", f.line),
		(FunctionKind::Script, None) => name.to_string(),
	}
}

/// A native's label without the crate paths' noise.
fn shorten_native(name: &str) -> String {
	name.replace("::{{closure}}", "").replace("squirrel_bb::native::", "native::")
}

/// A Markdown section titled `title` on profile `p`, its figures a frame's (`p.runs`
/// runs, one a frame), with `notes` (Markdown lines) under the title. Tables list the
/// `top` heaviest.
pub fn report(title: &str, notes: &str, p: &Profile, owners: &Owners, top: usize) -> String {
	let frames = p.runs.max(1);
	let per = |ns: u64| ns as f64 / 1e6 / frames as f64;
	let mut s = String::new();
	let _ = writeln!(s, "## {title}\n\n{notes}");
	let _ = writeln!(
		s,
		"- {frames} frames profiled: {:.3} ms a frame measured, {:.3} ms more the profiler's own work",
		per(p.wall_ns),
		per(p.overhead_ns)
	);
	let script_ns: u64 = p.nodes.iter().map(|n| n.self_ns).sum();
	let _ = writeln!(
		s,
		"- in functions: {:.3} ms ({:.3} ms outside any: the engine's own frame work); {:.0} call/return events a frame; {} unmatched",
		per(script_ns),
		per(p.outside_ns),
		p.events as f64 / frames as f64,
		p.unmatched
	);
	s.push('\n');

	// Owners of each node, and the time spent in each owner's code (self) and in calls
	// into it counted once per outermost entry (inclusive).
	let node_owner: Vec<&str> = p.nodes.iter().map(|n| owner(&p.functions[n.function], owners)).collect();
	let mut by_owner: HashMap<&str, (u64, u64, u64)> = HashMap::new();
	for (i, n) in p.nodes.iter().enumerate() {
		let o = node_owner[i];
		let entry = by_owner.entry(o).or_default();
		entry.0 += n.self_ns;
		entry.2 += n.calls;
		let mut up = n.parent;
		let mut inside = false;
		while let Some(u) = up {
			if node_owner[u] == o {
				inside = true;
				break;
			}
			up = p.nodes[u].parent;
		}
		if !inside {
			entry.1 += n.total_ns;
		}
	}
	let mut owners_sorted: Vec<_> = by_owner.into_iter().collect();
	owners_sorted.sort_by(|a, b| b.1.0.cmp(&a.1.0));
	let _ = writeln!(s, "### By owner (ms a frame)\n\n| owner | self | inclusive | calls |\n|---|---:|---:|---:|");
	for (o, (own, incl, calls)) in &owners_sorted {
		let _ = writeln!(s, "| {o} | {:.3} | {:.3} | {:.0} |", per(*own), per(*incl), *calls as f64 / frames as f64);
	}
	s.push('\n');

	let mut functions: Vec<&ProfiledFunction> = p.functions.iter().collect();
	functions.sort_by(|a, b| b.self_ns.cmp(&a.self_ns));
	table(&mut s, "Top functions by own time", &functions[..functions.len().min(top)], owners, frames);
	functions.sort_by(|a, b| b.total_ns.cmp(&a.total_ns));
	table(&mut s, "Top functions by inclusive time", &functions[..functions.len().min(top)], owners, frames);

	natives(&mut s, &p.functions, frames, top);

	let mut files: HashMap<String, (u64, u64)> = HashMap::new();
	for f in &p.functions {
		let key = match f.kind {
			FunctionKind::Native => NATIVE.to_string(),
			FunctionKind::Script => f.source.clone().unwrap_or_else(|| "?".into()),
		};
		let e = files.entry(key).or_default();
		e.0 += f.self_ns;
		e.1 += f.calls;
	}
	let mut files: Vec<_> = files.into_iter().collect();
	files.sort_by(|a, b| b.1.0.cmp(&a.1.0));
	let _ = writeln!(s, "### Top files by own time (ms a frame)\n\n| file | self | calls |\n|---|---:|---:|");
	for (file, (own, calls)) in files.iter().take(top) {
		let _ = writeln!(s, "| {file} | {:.3} | {:.1} |", per(*own), *calls as f64 / frames as f64);
	}
	s.push('\n');

	// The call tree, heaviest first.
	let mut children: HashMap<Option<usize>, Vec<usize>> = HashMap::new();
	for (i, n) in p.nodes.iter().enumerate() {
		children.entry(n.parent).or_default().push(i);
	}
	for list in children.values_mut() {
		list.sort_by(|a, b| p.nodes[*b].total_ns.cmp(&p.nodes[*a].total_ns));
	}
	let _ = writeln!(
		s,
		"### Call tree (paths ≥ {TREE_MIN_MS} ms a frame: inclusive / own ms, calls a frame)\n\n```"
	);
	let mut stack: Vec<(usize, usize)> = children.get(&None).map_or(Vec::new(), |r| r.iter().rev().map(|&n| (n, 0)).collect());
	while let Some((node, depth)) = stack.pop() {
		let n = &p.nodes[node];
		if per(n.total_ns) < TREE_MIN_MS {
			continue;
		}
		let f = &p.functions[n.function];
		let _ = writeln!(
			s,
			"{}{:.3} / {:.3}  {:>6.1}x  {}  [{}]",
			"  ".repeat(depth),
			per(n.total_ns),
			per(n.self_ns),
			n.calls as f64 / frames as f64,
			label(f),
			node_owner[node]
		);
		if depth < TREE_MAX_DEPTH {
			if let Some(kids) = children.get(&Some(node)) {
				stack.extend(kids.iter().rev().map(|&k| (k, depth + 1)));
			}
		}
	}
	s.push_str("```\n\n");
	s
}

fn table(s: &mut String, title: &str, functions: &[&ProfiledFunction], owners: &Owners, frames: u64) {
	let per = |ns: u64| ns as f64 / 1e6 / frames as f64;
	let _ = writeln!(s, "### {title} (ms a frame)\n\n| function | owner | calls | self | inclusive |\n|---|---|---:|---:|---:|");
	for f in functions {
		let _ = writeln!(
			s,
			"| {} | {} | {:.1} | {:.4} | {:.4} |",
			label(f).replace('|', "\\|"),
			owner(f, owners),
			f.calls as f64 / frames as f64,
			per(f.self_ns),
			per(f.total_ns)
		);
	}
	s.push('\n');
}

/// The engine's natives by own time: calls a frame, ms a frame, and ns a call (the
/// profiler's dispatch to the hook, ~40 ns, is in each call's figure).
fn natives(s: &mut String, functions: &[ProfiledFunction], frames: u64, top: usize) {
	let mut list: Vec<&ProfiledFunction> = functions.iter().filter(|f| f.kind == FunctionKind::Native).collect();
	list.sort_by(|a, b| b.self_ns.cmp(&a.self_ns));
	let total: u64 = list.iter().map(|f| f.self_ns).sum();
	let calls: u64 = list.iter().map(|f| f.calls).sum();
	let _ = writeln!(
		s,
		"### Natives by own time ({:.3} ms a frame, {:.0} calls a frame)\n\n| native | calls a frame | own ms a frame | ns a call |\n|---|---:|---:|---:|",
		total as f64 / 1e6 / frames as f64,
		calls as f64 / frames as f64
	);
	for f in list.iter().take(top) {
		let _ = writeln!(
			s,
			"| {} | {:.1} | {:.4} | {:.0} |",
			label(f).replace('|', "\\|"),
			f.calls as f64 / frames as f64,
			f.self_ns as f64 / 1e6 / frames as f64,
			f.self_ns as f64 / f.calls.max(1) as f64
		);
	}
	s.push('\n');
}
