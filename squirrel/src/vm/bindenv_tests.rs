//! `bindenv` hands out a closure it made earlier for the same binding when nothing but
//! the VM's cache holds it (squirrel's `sq_bindenv`). Scripts must not be able to tell:
//! every result they hold is a distinct closure, weak references die when they would
//! have, a changed root isn't handed out again, and garbage collection counts the same.

use crate::SQVM;

#[test]
fn bindenv_results_behave_as_fresh_closures() {
	let vm = SQVM::new(1024);
	vm.run_script(
		r#"
		local t = { n = 1, f = function() { return this.n; } };
		local u = { n = 2 };
		local f = t.f;

		// Held results are distinct closures.
		local a = f.bindenv(t), b = f.bindenv(t);
		assert(a != b);
		assert(a() == 1 && b() == 1);
		a = null; b = null;

		// Discarded results: each binding still calls with its own environment.
		local s = 0;
		for (local i = 0; i < 10; i++) s += f.bindenv(i % 2 ? t : u)();
		assert(s == 15);
		for (local i = 0; i < 10; i++) s += t.f.bindenv(t)();
		assert(s == 25);

		// A weak reference to a discarded result dies with it, once the stack slots that
		// held it are reused. (Checked without a call: `assert(…)`'s own frame keeps a
		// slot that held it alive.)
		local scrub = function() { local a = 0, b = 0, c = 0, d = 0, e = 0, f = 0, g = 0, h = 0; };
		local w = (function() { return f.bindenv(t).weakref(); })();
		scrub();
		if (w.ref() != null) throw "a weak reference to a discarded result is alive";
		local w2 = (function() { local g = f.bindenv(t); g(); return g.weakref(); })();
		scrub();
		if (w2.ref() != null) throw "a weak reference to a discarded result is alive";

		// A result whose root was changed isn't handed out again.
		local root = { n = 3 };
		(function() { local g = f.bindenv(t); g.setroot(root); })();
		assert(f.bindenv(t).getroot() == ::getroottable());

		// A result stays bound to its environment, which it holds weakly.
		local env = { n = 4 };
		local h = f.bindenv(env);
		assert(h() == 4);
		h = null;
		env = null;
		local k = f.bindenv(u);
		assert(k() == 2);

		// Collections see no object the cache kept: a cycle holding a result, and a
		// discarded result, collect as without the cache (the cycle's two tables; the
		// closure is freed when they are).
		collectgarbage();
		(function() {
			local x = {}, y = {};
			x.y <- y; y.x <- x;
			x.g <- f.bindenv(t);
			f.bindenv(t);
		})();
		scrub();
		local n = collectgarbage();
		if (n != 2) throw "collected " + n;
		"#,
		"bindenv_tests",
	)
	.expect("bindenv's results behave as fresh closures");
}
