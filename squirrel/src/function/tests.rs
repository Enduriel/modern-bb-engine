use std::cell::RefCell;
use std::rc::Rc;

use super::{StackArg, This, Variadic};
use crate::{SQVM, SqObjectType, SqStrongRef, SqTable};

#[test]
fn vm_context_param_gives_high_level_access() {
	let mut vm = SQVM::new(1024);

	// A native fn that takes the VM context (`&SQVM`) as its leading parameter,
	// then builds a table and stores it as a global — entirely through the public
	// high-level API, never touching the stack.
	vm.register("make_player", |vm: &SQVM, level: i64| {
		let mut player = SqTable::new(vm);
		player.set("level", level);
		SqTable::root(vm).set("Player", player);
	});
	// And one with a variadic tail, to exercise the Vm + variadic family.
	vm.register("sum_into", |vm: &SQVM, key: String, rest: Variadic<i64>| {
		SqTable::root(vm).set(&key, rest.0.into_iter().sum::<i64>());
	});

	vm.execute_squirrel_script(
		r#"
		make_player(7);
		assert(Player.level == 7);
		sum_into("total", 1, 2, 3);
		assert(total == 6);
		"#,
	)
	.expect("native fns built globals via the VM context");

	// `&SQVM` leading does not consume an argument slot: schema is `this` + fixed.
	vm.execute_squirrel_script(r#"assert(make_player.getinfos().paramscheck == 2);"#)
		.expect("schema unchanged by the &SQVM context param");
}

#[test]
fn make_closure_places_a_function_on_a_subtable() {
	let mut vm = SQVM::new(1024);
	// Build a native function as a value and slot it onto a sub-table (the way
	// the engine puts functions on IO / Math / World rather than the root).
	let add = vm.make_closure(|a: i64, b: i64| a + b);
	let mut lib = SqTable::new(&vm);
	lib.set("add", add);
	SqTable::root(&vm).set("Lib", lib);

	vm.execute_squirrel_script(r#"assert(Lib.add(2, 3) == 5);"#)
		.expect("sub-table function is callable");
}

#[test]
fn native_fn_accepts_a_table_argument() {
	let mut vm = SQVM::new(1024);
	// A borrowed `SqTable` parameter reads straight from the argument on the stack.
	vm.register("hp_of", |t: SqTable| t.get::<i64>("hp").unwrap_or(0));

	vm.execute_squirrel_script(r#"assert(hp_of({ hp = 42 }) == 42);"#)
		.expect("table argument is read");
	// The `t` type-mask character makes Squirrel reject a non-table argument.
	vm.execute_squirrel_script(r#"hp_of(5);"#)
		.expect_err("an integer is not a table");
}

#[test]
fn native_fn_returns_a_built_table() {
	let mut vm = SQVM::new(1024);
	// Build a table and return it to Squirrel as an owned handle.
	vm.register("make", |vm: &SQVM, x: i64| -> SqStrongRef {
		let mut t = SqTable::new(vm);
		t.set("x", x);
		t.set("doubled", x * 2);
		t.to_strong()
	});

	vm.execute_squirrel_script(
		r#"
		local t = make(21);
		assert(t.x == 21);
		assert(t.doubled == 42);
		"#,
	)
	.expect("returned table is usable in script");
}

#[test]
fn register_closure_basic_calls() {
	let mut vm = SQVM::new(1024);

	vm.register("add", |a: i64, b: i64| a + b);
	vm.register("greet", |name: String| format!("hello {name}"));
	vm.register("toggle", |b: bool| !b);
	vm.register("half", |x: f64| x / 2.0);
	vm.register("noop", || {});

	vm.execute_squirrel_script(
		r#"
		assert(add(2, 3) == 5);
		assert(greet("bob") == "hello bob");
		assert(toggle(true) == false);
		assert(half(5.0) == 2.5);
		noop();
		"#,
	)
	.expect("registered closures should run");
}

#[test]
fn register_derives_paramscheck_schema() {
	let mut vm = SQVM::new(1024);

	// `|String, i64|` derives the mask ".s." (`this` -> '.', String -> 's', i64 ->
	// '.': numeric args accept any value and coerce in `from_sq`) and an exact count
	// of 3 (`this` + two params), observable script-side via `getinfos()`.
	vm.register("checked", |_s: String, _i: i64| 0i64);

	vm.execute_squirrel_script(
		r#"
		local info = checked.getinfos();
		assert(info.native == true);
		assert(info.paramscheck == 3);
		assert(info.typecheck != null);
		assert(info.typecheck.len() == 3);
		"#,
	)
	.expect("derived schema assertions should pass");

	vm.execute_squirrel_script(r#"checked("hi", 1);"#)
		.expect("valid call should succeed");
	vm.execute_squirrel_script(r#"checked("hi");"#)
		.expect_err("missing argument should be rejected");
	vm.execute_squirrel_script(r#"checked("hi", 1, 2);"#)
		.expect_err("extra argument should be rejected");
	vm.execute_squirrel_script(r#"checked(1, 2);"#)
		.expect_err("type mismatch should be rejected");
}

#[test]
fn numeric_args_coerce_between_int_and_float() {
	let mut vm = SQVM::new(1024);

	// An `i64` parameter accepts a float and truncates it; an `f64` parameter accepts
	// an integer and widens it — mirroring `sq_getinteger`/`sq_getfloat` and the way
	// BB passes floats to integer natives (e.g. `getTileSquare` with a float coord).
	vm.register("takes_int", |i: i64| i);
	vm.register("takes_float", |f: f64| f);

	vm.execute_squirrel_script(
		r#"
		assert(takes_int(3) == 3);
		assert(takes_int(3.9) == 3);   // float argument truncated to int
		assert(takes_float(5) == 5.0); // int argument widened to float
		assert(takes_float(5.5) == 5.5);
		"#,
	)
	.expect("numeric coercion should succeed");

	// A non-numeric argument is still rejected — now inside `from_sq` rather than by
	// the (now permissive `.`) type mask.
	vm.execute_squirrel_script(r#"takes_int("nope");"#)
		.expect_err("non-numeric argument should be rejected");
}

#[test]
fn register_variadic_collects_trailing_args() {
	let mut vm = SQVM::new(1024);

	// Fixed string label, then any number of trailing integers.
	vm.register("sum", |_label: String, rest: Variadic<i64>| {
		rest.0.into_iter().sum::<i64>()
	});

	vm.execute_squirrel_script(
		r#"
		assert(sum.getinfos().paramscheck == -2);
		assert(sum("nums") == 0);
		assert(sum("nums", 1, 2, 3) == 6);
		"#,
	)
	.expect("variadic schema and calls should work");

	vm.execute_squirrel_script(r#"sum();"#)
		.expect_err("below minimum arity should be rejected");
	vm.execute_squirrel_script(r#"sum(42);"#)
		.expect_err("wrong fixed-param type should be rejected");
	// The type mask only covers the fixed params, so a bad variadic element
	// is caught Rust-side during conversion.
	vm.execute_squirrel_script(r#"sum("nums", 1, "x");"#)
		.expect_err("non-integer trailing arg should be rejected");
}

#[test]
fn register_supports_state_results_and_options() {
	let mut vm = SQVM::new(1024);

	// Stateful `FnMut`: the captured counter persists across calls.
	let calls = Rc::new(RefCell::new(0i64));
	let calls_in = calls.clone();
	vm.register("tick", move |step: i64| {
		*calls_in.borrow_mut() += step;
		*calls_in.borrow()
	});

	// `Result::Err` is raised as a Squirrel exception.
	vm.register("checked_div", |a: i64, b: i64| -> Result<i64, String> {
		if b == 0 {
			Err("division by zero".to_string())
		} else {
			Ok(a / b)
		}
	});

	// `Option` arg accepts null; `Option` return maps `None` -> null.
	vm.register("default_or", |x: Option<i64>| x.unwrap_or(-1));

	vm.execute_squirrel_script(
		r#"
		assert(tick(2) == 2);
		assert(tick(3) == 5);
		assert(checked_div(10, 2) == 5);
		assert(default_or(7) == 7);
		assert(default_or(null) == -1);
		"#,
	)
	.expect("registered closures should run");

	vm.execute_squirrel_script(r#"checked_div(1, 0);"#)
		.expect_err("Err return should raise a Squirrel exception");

	// State persisted across the two `tick` calls above: 2 + 3 == 5.
	assert_eq!(*calls.borrow(), 5);
}

#[test]
fn this_does_not_change_param_schema() {
	let mut vm = SQVM::new(1024);

	// Identical signature apart from the leading `This`: the schema Squirrel
	// sees must be byte-identical (no phantom param, no extra typemask char).
	vm.register("with_this", |_this: This, _a: i64| 0i64);
	vm.register("without_this", |_a: i64| 0i64);

	vm.execute_squirrel_script(
		r#"
		local w = with_this.getinfos();
		local n = without_this.getinfos();
		assert(w.paramscheck == 2);
		assert(n.paramscheck == 2);
		assert(w.paramscheck == n.paramscheck);
		assert(w.typecheck.len() == 2);
		assert(w.typecheck.len() == n.typecheck.len());
		"#,
	)
	.expect("`This` must not alter the param-check schema");
}

#[test]
fn this_reports_receiver_type() {
	let mut vm = SQVM::new(1024);

	// Return a discriminant for the receiver's runtime type.
	vm.register("kind", |this: This| match this.object_type() {
		SqObjectType::Table => 1i64,
		SqObjectType::Instance => 2,
		SqObjectType::Class => 3,
		_ => 0,
	});

	vm.execute_squirrel_script(
		r#"
		// Free call: the receiver is the root table.
		assert(kind() == 1);
		// Explicit receiver via closure.call(this): a fresh table, then an instance.
		class Foo {}
		local inst = Foo();
		assert(kind.call({}) == 1);
		assert(kind.call(inst) == 2);
		"#,
	)
	.expect("object_type should reflect the actual receiver");
}

#[test]
fn this_does_not_shift_arg_indices() {
	let mut vm = SQVM::new(1024);

	// Extracting `this` (index 1) must not shift the args, which still begin
	// at index 2 — `a` from slot 2, `b` from slot 3.
	vm.register(
		"pick",
		|_this: This, a: i64, b: String| {
			if a == 7 { b } else { "no".to_string() }
		},
	);

	vm.execute_squirrel_script(
		r#"
		assert(pick.getinfos().paramscheck == 3);
		assert(pick(7, "ok") == "ok");
		assert(pick(3, "ok") == "no");
		"#,
	)
	.expect("args after `This` are read from slot 2 onward");
}

#[test]
fn this_with_variadic_tail() {
	let mut vm = SQVM::new(1024);

	// `This` + pure variadic.
	vm.register("vsum", |_this: This, rest: Variadic<i64>| {
		rest.0.into_iter().sum::<i64>()
	});
	// `This` + one fixed param + variadic.
	vm.register(
		"lsum",
		|_this: This, _label: String, rest: Variadic<i64>| rest.0.into_iter().sum::<i64>(),
	);

	vm.execute_squirrel_script(
		r#"
		assert(vsum.getinfos().paramscheck == -1);
		assert(vsum() == 0);
		assert(vsum(1, 2, 3) == 6);

		assert(lsum.getinfos().paramscheck == -2);
		assert(lsum("x") == 0);
		assert(lsum("x", 4, 5) == 9);
		"#,
	)
	.expect("`This` composes with a variadic tail");
}

#[test]
fn this_only_no_args() {
	let mut vm = SQVM::new(1024);

	let called = Rc::new(RefCell::new(0i64));
	let called_in = called.clone();
	vm.register("ping", move |_this: This| {
		*called_in.borrow_mut() += 1;
	});

	vm.execute_squirrel_script(
		r#"
		assert(ping.getinfos().paramscheck == 1);
		ping();          // free call
		ping.call({});   // explicit receiver
		"#,
	)
	.expect("a `This`-only no-arg method should run");

	assert_eq!(*called.borrow(), 2);
}

#[test]
fn register_borrowed_str_args() {
	let mut vm = SQVM::new(1024);

	// Borrowed `&str`: reads straight from the stack, no per-call allocation.
	vm.register("first_char", |s: &str| {
		s.chars().next().map(|c| c as i64).unwrap_or(-1)
	});
	// Borrowed `&str` alongside an owned arg, in either position.
	vm.register("len_plus", |s: &str, n: i64| s.len() as i64 + n);
	vm.register(
		"tagged",
		|n: i64, s: &str| if n > 0 { s.len() as i64 } else { 0 },
	);
	// Two borrowed strings at once.
	vm.register("concat_len", |a: &str, b: &str| (a.len() + b.len()) as i64);

	vm.execute_squirrel_script(
		r#"
		assert(first_char("hello") == 'h');   // 'h' is its integer code point
		assert(len_plus("abc", 2) == 5);
		assert(tagged(1, "xyz") == 3);
		assert(tagged(-1, "xyz") == 0);
		assert(concat_len("ab", "cde") == 5);
		// `&str` carries the same 's' type mask as `String`.
		assert(first_char.getinfos().paramscheck == 2);
		assert(first_char.getinfos().typecheck.len() == 2);
		"#,
	)
	.expect("borrowed `&str` args should work");

	// Wrong type is still rejected by the type mask.
	vm.execute_squirrel_script(r#"first_char(42);"#)
		.expect_err("non-string should be rejected");
}

#[test]
fn register_optional_and_this_borrowed_str() {
	let mut vm = SQVM::new(1024);

	// `Option<&str>` accepts a borrowed string or null.
	vm.register("len_or", |x: Option<&str>| {
		x.map(|s| s.len() as i64).unwrap_or(-1)
	});
	// `This` + borrowed `&str`.
	vm.register("echo_len", |_this: This, s: &str| s.len() as i64);
	// Variadic tail of borrowed strings.
	vm.register("total_len", |_label: &str, rest: Variadic<&str>| {
		rest.0.iter().map(|s| s.len()).sum::<usize>() as i64
	});

	vm.execute_squirrel_script(
		r#"
		assert(len_or("hi") == 2);
		assert(len_or(null) == -1);
		assert(echo_len("hello") == 5);
		assert(total_len("tag") == 0);
		assert(total_len("tag", "a", "bc", "def") == 6);
		"#,
	)
	.expect("Option<&str>, This+&str, and Variadic<&str> should work");
}

/// A dispatcher built on `forward_member`: picks `leaf_<arity>` on the receiver and
/// forwards the call's own argument slots to it, returning the leaf's result in place.
fn register_forwarding_dispatcher(vm: &mut SQVM) {
	let dispatch = vm.make_closure(|vm: &SQVM, this: This, args: Variadic<StackArg>| {
		let key = format!("leaf_{}", args.0.len());
		match this.forward_member(vm, &key, &args.0)? {
			Some(result) => Ok(result),
			None => Err("no such leaf".to_string()),
		}
	});
	let mut obj = SqTable::new(vm);
	obj.set("go", dispatch);
	SqTable::root(vm).set("Obj", obj);
}

#[test]
fn forward_member_passes_args_through_and_returns_the_result() {
	let mut vm = SQVM::new(1024);
	register_forwarding_dispatcher(&mut vm);
	vm.run_script(
		r#"
		Obj.leaf_0 <- function() { return this == ::Obj ? "zero" : "wrong this"; };
		Obj.leaf_2 <- function(a, b) { return a - b; };               // order matters
		Obj.leaf_3 <- function(t, s, f) { return t.x + s.len() + f; }; // any types
		Obj.leaf_1 <- function(x) { return null; };
		assert(Obj.go() == "zero");
		assert(Obj.go(10, 3) == 7);
		assert(Obj.go({ x = 1 }, "ab", 0.5) == 3.5);
		assert(Obj.go(5) == null);
		local table = {};
		Obj.leaf_1 = function(x) { return x; };
		assert(Obj.go(table) == table);                               // same object back
		local n = 0;
		for (local i = 0; i < 10000; i++) n += Obj.go(i, 1);          // stack stays balanced
		assert(n == 49985000);
		"#,
		"forward_member",
	)
	.expect("forwarded calls behave like direct ones");
}

#[test]
fn forward_member_reports_a_missing_member_and_propagates_errors() {
	let mut vm = SQVM::new(1024);
	register_forwarding_dispatcher(&mut vm);
	vm.run_script(
		r#"
		Obj.leaf_1 <- function(x) { throw "leaf failed: " + x; };
		local missing = null, raised = null;
		try { Obj.go(1, 2, 3, 4); } catch (e) { missing = e; }
		try { Obj.go("boom"); } catch (e) { raised = e; }
		assert(missing == "no such leaf");
		assert(raised.find("leaf failed: boom") != null);
		"#,
		"forward_member_errors",
	)
	.expect("a missing member and a raising member both surface as errors");
	let uncaught = vm.run_script("Obj.go(1, 2, 3, 4, 5);", "forward_member_uncaught");
	assert!(uncaught.unwrap_err().contains("no such leaf"));
}

/// What the standard error handler prints for an uncaught error in `closure`.
fn uncaught_report(vm: &SQVM, closure: &SqStrongRef, out: &Rc<RefCell<String>>) -> String {
	out.borrow_mut().clear();
	assert!(closure.call(vm, ()).is_err());
	out.borrow().clone()
}

#[test]
fn write_closure_round_trips_with_source_names_and_lines() {
	let mut vm = SQVM::new(1024);
	let out = Rc::new(RefCell::new(String::new()));
	let (sink, err_sink) = (out.clone(), out.clone());
	vm.set_print_handler(move |s| sink.borrow_mut().push_str(s));
	vm.set_error_handler(move |s| err_sink.borrow_mut().push_str(s));
	vm.install_standard_error_handlers();

	let ok = vm
		.compile("local x = 40;\nreturn x + 2;", "ok.nut")
		.unwrap();
	let reloaded = vm.load_closure(&ok.write_closure(&vm).unwrap()).unwrap();
	assert_eq!(reloaded.call_ret::<i64, _>(&vm, ()).unwrap(), Some(42));

	let bad = vm
		.compile(
			"local a = 1;\nlocal f = function() {\n\treturn null.c;\n};\nreturn f();",
			"bad.nut",
		)
		.unwrap();
	let original = uncaught_report(&vm, &bad, &out);
	let reloaded = vm.load_closure(&bad.write_closure(&vm).unwrap()).unwrap();
	assert_eq!(uncaught_report(&vm, &reloaded, &out), original);
	assert!(
		original.contains("bad.nut") && original.contains("line [3]"),
		"{original}"
	);

	let native = vm.make_closure(|| 1i64);
	assert!(
		native.write_closure(&vm).is_err(),
		"a native closure cannot be serialized"
	);
}
