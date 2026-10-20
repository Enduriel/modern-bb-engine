use crate::{FunctionKind, Profile, SQVM, ScriptProfiler};

fn profile(vm: &mut SQVM, source: &str) -> Profile {
	let mut profiler = ScriptProfiler::new();
	vm.profiled(&mut profiler, || vm.run_script(source, "test.nut").unwrap());
	profiler.profile()
}

fn calls(p: &Profile, name: &str) -> u64 {
	p.functions.iter().filter(|f| f.name.as_deref() == Some(name)).map(|f| f.calls).sum()
}

/// The path of the node: function names from the top.
fn path(p: &Profile, mut node: usize) -> String {
	let mut names = Vec::new();
	loop {
		let n = &p.nodes[node];
		names.push(p.functions[n.function].name.clone().unwrap_or_else(|| "?".into()));
		match n.parent {
			Some(up) => node = up,
			None => break,
		}
	}
	names.reverse();
	names.join(">")
}

#[test]
fn counts_calls_and_builds_the_tree() {
	let mut vm = SQVM::new(1024);
	let p = profile(
		&mut vm,
		"function leaf(x) { return x + 1; }
		 function middle() { local s = 0; for (local i = 0; i < 3; i++) s += leaf(i); return s; }
		 for (local i = 0; i < 5; i++) middle();",
	);
	assert_eq!(calls(&p, "middle"), 5);
	assert_eq!(calls(&p, "leaf"), 15);
	let paths: Vec<String> = (0..p.nodes.len()).map(|n| path(&p, n)).collect();
	assert!(paths.iter().any(|s| s.ends_with("middle>leaf")), "{paths:?}");
	let leaf = p.functions.iter().find(|f| f.name.as_deref() == Some("leaf")).unwrap();
	assert_eq!(leaf.source.as_deref(), Some("test.nut"));
	assert_eq!(leaf.kind, FunctionKind::Script);
	assert_eq!(p.unmatched, 0);
	assert_eq!(p.runs, 1);
}

#[test]
fn times_add_up() {
	let mut vm = SQVM::new(1024);
	let p = profile(
		&mut vm,
		"function work() { local s = 0; for (local i = 0; i < 20000; i++) s += i; return s; }
		 function outer() { return work() + work(); }
		 outer();",
	);
	let f = |name: &str| p.functions.iter().find(|f| f.name.as_deref() == Some(name)).unwrap().clone();
	let (work, outer) = (f("work"), f("outer"));
	assert!(work.self_ns > 0 && work.total_ns == work.self_ns, "{work:?}");
	assert!(outer.total_ns >= work.total_ns + outer.self_ns, "{outer:?} {work:?}");
	let selves: u64 = p.nodes.iter().map(|n| n.self_ns).sum::<u64>() + p.outside_ns;
	assert_eq!(selves, p.wall_ns);
}

#[test]
fn tail_calls_and_errors_close_their_calls() {
	let mut vm = SQVM::new(1024);
	let p = profile(
		&mut vm,
		"function c() { return 1; }
		 function b() { return c(); }
		 function a() { return b(); }
		 function thrower() { throw \"no\"; }
		 function catcher() { try { thrower(); } catch (e) {} return 2; }
		 for (local i = 0; i < 4; i++) { a(); catcher(); }",
	);
	assert_eq!((calls(&p, "a"), calls(&p, "b"), calls(&p, "c")), (4, 4, 4));
	assert_eq!((calls(&p, "catcher"), calls(&p, "thrower")), (4, 4));
	// Each loop's calls hang off the top level, not under an unclosed call.
	let paths: Vec<String> = (0..p.nodes.len()).map(|n| path(&p, n)).collect();
	assert!(paths.iter().any(|s| s.ends_with("main>catcher>thrower")), "{paths:?}");
	assert!(paths.iter().any(|s| s.ends_with("main>a>b>c")), "{paths:?}");
	assert!(!paths.iter().any(|s| s.contains("a>b>c>catcher")), "{paths:?}");
}

#[test]
fn natives_are_profiled_with_their_script_callees() {
	let mut vm = SQVM::new(1024);
	vm.register("ignore", |f: crate::SqStrongRef| drop(f));
	let twice = vm.make_closure(|vm: &SQVM, f: crate::SqStrongRef| {
		f.call(vm, ()).unwrap();
		f.call(vm, ()).unwrap();
	});
	crate::SqTable::root(&vm).set("twice", twice);
	let p = profile(&mut vm, "function inner() { return 1; } twice(inner); ignore(inner);");
	let native: Vec<_> = p.functions.iter().filter(|f| f.kind == FunctionKind::Native).collect();
	assert_eq!(native.len(), 2, "{native:?}");
	assert!(native.iter().all(|f| f.calls == 1 && f.name.as_deref().unwrap().contains("profile::tests")), "{native:?}");
	assert_eq!(calls(&p, "inner"), 2);
	let paths: Vec<String> = (0..p.nodes.len()).map(|n| path(&p, n)).collect();
	assert!(paths.iter().any(|s| s.ends_with(">inner") && s.contains("{{closure}}")), "{paths:?}");
}

#[test]
fn off_by_default_and_detached_after() {
	let vm = SQVM::new(1024);
	let mut profiler = ScriptProfiler::new();
	vm.run_script("function f() {} f();", "before.nut").unwrap();
	vm.profiled(&mut profiler, || vm.run_script("f();", "during.nut").unwrap());
	vm.run_script("f(); f();", "after.nut").unwrap();
	let p = profiler.profile();
	assert_eq!(calls(&p, "f"), 1);
	// Nested profiling just runs.
	let mut inner = ScriptProfiler::new();
	vm.profiled(&mut profiler, || vm.profiled(&mut inner, || vm.run_script("f();", "nested.nut").unwrap()));
	assert_eq!(calls(&profiler.profile(), "f"), 2);
	assert_eq!(inner.profile().runs, 0);
}
