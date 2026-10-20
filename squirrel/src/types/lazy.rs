//! Lazy clones of tables: a table that behaves exactly as a clone of its source would
//! (and, once bound, as that clone with every closure bound to an object), but holds only
//! the slots that were read or written. Iterating it, cloning it or deleting from it first
//! turns it into the plain table it stands for, laid out as the clone would be; a write to
//! the source after the clone isn't seen through it. The BB class system builds an
//! instance's class levels this way, so the inherited methods an instance never touches
//! cost it nothing.

use super::SqTable;
use crate::SQ_OK;

impl<'vm> SqTable<'vm> {
	/// A lazy clone of this table: the same slots and delegate as
	/// [`shallow_clone`](Self::shallow_clone) gives, read through on demand.
	pub fn lazy_clone(&self) -> SqTable<'vm> {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [self]
			let ok = squirrel_sys::sq_lazyclone(ptr, -1) == SQ_OK; // [self, clone]
			let clone = ok.then(|| SqTable::from_stack(self.vm, -1).expect("a lazy clone is a table"));
			squirrel_sys::sq_pop(ptr, if ok { 2 } else { 1 });
			clone.expect("the source is a table")
		}
	}

	/// For a [`lazy_clone`](Self::lazy_clone): from now on the closures it reads from its
	/// source come bound to `env`, as [`bind_closures_to`](Self::bind_closures_to) would
	/// have bound them in a plain clone (each bound once, when first read, and kept). Does
	/// nothing to a plain table.
	pub fn lazy_bind_to(&mut self, env: &SqTable) {
		let ptr = self.vm.as_ptr();
		unsafe {
			squirrel_sys::sq_pushobject(ptr, self.obj); // [self]
			squirrel_sys::sq_pushobject(ptr, env.obj); // [self, env]
			if squirrel_sys::sq_lazybind(ptr, -2) == SQ_OK {
				squirrel_sys::sq_pop(ptr, 1); // the env was popped -> []
			} else {
				squirrel_sys::sq_reseterror(ptr);
				squirrel_sys::sq_pop(ptr, 2);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use crate::{SQVM, SqStrongRef, SqTable};

	/// A VM with `lazyclone(t)`, `lazybind(t, env)` and `eagerbind(t, env)` (a plain
	/// table's closures bound, as the class system binds a clone's).
	fn vm() -> SQVM {
		let mut vm = SQVM::new(1024);
		vm.register("lazyclone", |vm: &SQVM, t: SqTable| -> SqStrongRef {
			let _ = vm;
			t.lazy_clone().to_strong()
		});
		vm.register("lazybind", |vm: &SQVM, mut t: SqTable, env: SqTable| {
			let _ = vm;
			t.lazy_bind_to(&env);
		});
		vm.register("eagerbind", |vm: &SQVM, mut t: SqTable, env: SqTable| {
			let _ = vm;
			t.bind_closures_to(&env);
		});
		vm
	}

	/// Random operations on a plain clone (bound) and a lazy one (bound) of the same
	/// table, and on the source after the clones were made: everything a script can see
	/// of the two must be the same, iteration order included.
	const DIFFERENTIAL: &str = r##"
	::seed <- 1;
	::start <- 0;
	function rnd(n) { ::seed = (::seed * 1103515245 + 12345) & 0x7fffffff; return (::seed >> 8) % n; }
	local shared = { s = 1 };
	local fns = [];
	for (local i = 0; i < 6; i++) { local tag = "f" + i; fns.push(function() { return this.id + tag; }); }
	function value() {
		switch (rnd(5)) {
			case 0: return rnd(100);
			case 1: return "v" + rnd(10);
			case 2: return shared;
			default: return fns[rnd(fns.len())];
		}
	}
	function key() { return rnd(8) == 0 ? rnd(4) : "k" + rnd(48); }
	function describe(v) {
		if (typeof v == "function") return "fn:" + v.call({ id = "plain" });
		if (typeof v == "table") return v == shared ? "shared" : "table";
		return typeof v + ":" + v;
	}
	function dump(t) {
		local s = "";
		foreach (k, v in t) s += k + "=" + describe(v) + ",";
		return s + "#" + t.len();
	}
	function check(what, a, b) {
		if (a != b) throw "seed " + ::start + ": " + what + ": " + a + " vs " + b + " after " + ::trace;
	}
	function run(start) {
		::start = start;
		::seed = start;
		::trace <- "";
		local env = { id = "leaf" };
		local p = {};
		for (local n = rnd(40); n > 0; n--) p[key()] <- value();
		local pairs = [];
		local e = clone p; eagerbind(e, env);
		local l = lazyclone(p); lazybind(l, env);
		pairs.push([e, l]);
		for (local step = 0; step < 60; step++) {
			local pr = pairs[rnd(pairs.len())];
			local e = pr[0], l = pr[1];
			local k = key();
			local op = rnd(12);
			::trace += " " + pairs.find(pr) + ":" + op + ":" + k;
			switch (op) {
				case 0:
					local a = k in e, b = k in l;
					check("in " + k, a, b);
					if (a) {
						check("read " + k, describe(e[k]), describe(l[k]));
						check("same object per read " + k, l[k] == l[k], true);
					}
					break;
				case 1:
					local v = value(), a = "ok", b = "ok";
					try { e[k] = v; } catch (x) { a = "error"; }
					try { l[k] = v; } catch (x) { b = "error"; }
					check("set " + k, a, b);
					break;
				case 2:
					local v = value();
					e[k] <- v; l[k] <- v;
					break;
				case 3:
					check("rawin " + k, e.rawin(k), l.rawin(k));
					check("len", e.len(), l.len());
					break;
				case 4:
					if (k in e) { check("delete " + k, describe(delete e[k]), describe(delete l[k])); }
					break;
				case 5:
					// The source changes after the clones: neither sees it.
					local v = value();
					switch (rnd(3)) {
						case 0: p[k] <- v; break;
						case 1: if (k in p) p[k] = v; break;
						case 2: if (k in p) delete p[k]; break;
					}
					// A clone made now sees the change, the same either way.
					if (rnd(4) == 0) {
						local e2 = clone p; eagerbind(e2, env);
						local l2 = lazyclone(p); lazybind(l2, env);
						pairs.push([e2, l2]);
					}
					break;
				case 6:
					check("iteration", dump(e), dump(l));
					break;
				case 7:
					check("clone", dump(clone e), dump(clone l));
					break;
				case 8:
					// A lazy clone of a lazy table, unbound (a class's copy of its ancestors).
					pairs.push([clone e, lazyclone(l)]);
					break;
				case 9:
					if (rnd(6) == 0) { e.clear(); l.clear(); }
					break;
				case 10:
					if (e.rawin(k)) check("rawget " + k, describe(e.rawget(k)), describe(l.rawget(k)));
					break;
				case 11:
					if (k in e) check("rawdelete " + k, describe(e.rawdelete(k)), describe(l.rawdelete(k)));
					break;
			}
		}
		foreach (pr in pairs) check("final iteration", dump(pr[0]), dump(pr[1]));
	}
	"##;

	#[test]
	fn a_lazy_clone_behaves_as_a_bound_clone() {
		let mut vm = vm();
		vm.install_standard_error_handlers();
		vm.run_script(DIFFERENTIAL, "differential").expect("define");
		for seed in 1..=3000 {
			vm.run_script(&format!("run({seed});"), "run").unwrap_or_else(|e| panic!("{e}"));
		}
		vm.collect_garbage();
	}

	#[test]
	fn a_bound_lazy_clone_binds_on_first_read() {
		let vm = vm();
		vm.run_script(
			r#"
			local proto = { name = "proto", function who() { return this.name; } };
			local leaf = { name = "leaf" };
			local l = lazyclone(proto);
			lazybind(l, leaf);
			assert(l.who() == "leaf");
			assert(l.who == l.who);
			assert(l.who != proto.who);
			assert(l.who.call({ name = "x" }) == "leaf");
			assert(l.name == "proto");
			// The source patched afterwards: the lazy clone keeps what it was cloned from.
			proto.who = function() { return "patched"; };
			proto.extra <- 1;
			local l2 = lazyclone(proto);
			assert(!("extra" in l) && ("extra" in l2));
			assert(lazyclone(proto).who() == "patched");
			assert(l.who() == "leaf");
			"#,
			"bind",
		)
		.expect("lazy binding");
	}

	/// The collector sees what a lazy table reads through: a source only it still
	/// reaches survives a collection, and cycles through lazy tables are freed.
	#[test]
	fn the_collector_follows_a_lazy_tables_source() {
		let vm = vm();
		vm.run_script(
			r#"
			local proto = { a = 1, b = { c = 2 }, function f() { return this.id; } };
			proto.self <- proto;
			::keep <- lazyclone(proto);
			lazybind(::keep, { id = "gone" });
			// The source is patched (the lazy table keeps a copy) and dropped.
			proto.a = 5;
			proto = null;
			::cycle <- lazyclone({ x = 1 });
			::cycle.me <- ::cycle;
			::cycle = null;
			"#,
			"setup",
		)
		.unwrap();
		assert!(vm.collect_garbage() > 0, "the cycle through a lazy table is freed");
		vm.run_script(
			r#"
			assert(::keep.a == 1 && ::keep.b.c == 2 && ::keep.self.a == 5);
			local n = 0;
			foreach (k, v in ::keep) n++;
			assert(n == 4);
			"#,
			"after",
		)
		.expect("the source read through survived");
		vm.collect_garbage();
	}
}
