//! The VM's lookup cache (squirrel's `SQVM::ChainGet`) answers a lookup that
//! missed a table and its delegate from where it ended last time. Scripts must not be able
//! to tell: every change that could move the answer (a key given to or removed from a
//! table on the way, a delegate set, a `_get` added, a table freed and another made at its
//! address, another root) is seen at once, after the cache has been warmed.

use crate::SQVM;

#[test]
fn a_warmed_lookup_sees_every_change_on_its_way() {
	let vm = SQVM::new(1024);
	vm.run_script(
		r#"
		::G <- "root";
		local bottom = {}, mid = {}, leaf = {};
		mid.setdelegate(bottom);
		leaf.setdelegate(mid);
		local f = function() { return this.G; }.bindenv(leaf);
		local warm = function(want) { for (local i = 0; i < 3; i++) if (f() != want) throw "got " + f() + ", wanted " + want; };
		warm("root");

		// A key given to (and removed from) each table on the way.
		bottom.G <- "bottom"; warm("bottom");
		delete bottom.G; warm("root");
		mid.G <- "mid"; warm("mid");
		delete mid.G; warm("root");
		leaf.G <- "leaf"; warm("leaf");
		delete leaf.G; warm("root");
		// The root's value written.
		::G = "root2"; warm("root2");
		// A delegate's `_get`, then its clean failure (`throw null`) going on to the root.
		bottom._get <- function(k) { return "got " + k; };
		warm("got G");
		bottom._get = function(k) { throw null; };
		warm("root2");
		delete bottom._get; warm("root2");
		mid._get <- function(k) { return "mid got " + k; };
		warm("mid got G"); // as leaf's delegate
		delete mid._get;
		// Delegates set and cleared.
		local other = { G = "other" };
		mid.setdelegate(other); warm("other");
		mid.setdelegate(bottom); warm("root2");
		bottom.setdelegate(other); warm("other");
		bottom.setdelegate(null); warm("root2");
		bottom.clear(); bottom.G <- "bottom again"; warm("bottom again");
		bottom.clear(); warm("root2");

		// The default delegate answers before the root.
		local n = function() { return this.len(); }.bindenv(leaf);
		assert(n() == 0 && n() == 0);
		leaf.x <- 1;
		assert(n() == 1);
		::len <- function() { return -1; };
		assert(n() == 1);

		// Not through `this`: no root fallback, though the same lookup through `this`
		// was answered by the root.
		warm("root2");
		local o = leaf, caught = null;
		try { local v = o.G; } catch (e) { caught = e; }
		assert(caught != null);
		warm("root2");

		// Another root for a closure of the same function bound to the same table.
		local r2 = { G = "r2" };
		local g = f.bindenv(leaf);
		g.setroot(r2);
		for (local i = 0; i < 3; i++) assert(f() == "root2" && g() == "r2");
		delete r2.G;
		caught = null;
		try { g(); } catch (e) { caught = e; }
		assert(caught != null);
		assert(f() == "root2");
		"#,
		"lookup_cache",
	)
	.expect("the cache follows every change");
}

#[test]
fn a_table_freed_and_made_again_at_its_address_is_not_taken_for_it() {
	let vm = SQVM::new(1024);
	vm.run_script(
		r#"
		::K <- "root";
		for (local i = 0; i < 5000; i++) {
			local top = (i % 3 == 0) ? {} : { K = i };
			local mid = {}, leaf = {};
			mid.setdelegate(top);
			leaf.setdelegate(mid);
			local f = function() { return this.K; }.bindenv(leaf);
			local want = (i % 3 == 0) ? "root" : i;
			if (f() != want || f() != want) throw "iteration " + i + ": got " + f();
		}
		"#,
		"address_reuse",
	)
	.expect("freed tables are forgotten");
}

/// Random tables, delegate links, keys, `_get` metamethods and frees, with lookups through
/// `this` and from outside, run once with the cache and once without: the same log.
#[test]
fn random_changes_read_the_same_with_and_without_the_cache() {
	const SCRIPT: &str = r#"
	local seed = 12345;
	local function rnd(n) { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return (seed >> 8) % n; }
	local keys = ["a", "b", "c", "d", "len", "_get"];
	local tables = [];
	for (local i = 0; i < 12; i++) tables.push({});
	::a <- "root a"; ::c <- "root c";
	local getters = [
		function(k) { return "mm " + k; },
		function(k) { throw null; },
		function(k) { if (k == "b") return "mm b only"; throw null; }
	];
	local out = [];
	local function look(t, k) {
		local f = compilestring("return this." + k + ";").bindenv(t);
		local r;
		try { r = f(); } catch (e) { r = "error"; }
		if (typeof r == "function") r = "function";
		local s;
		try { s = t[k]; } catch (e) { s = "error"; }
		if (typeof s == "function") s = "function";
		return r + "/" + s;
	}
	local lookers = {};
	foreach (k in keys) lookers[k] <- [];
	for (local step = 0; step < 20000; step++) {
		local t = tables[rnd(tables.len())];
		local op = rnd(16);
		if (op == 0) { local k = keys[rnd(4)]; t[k] <- "v" + step; }
		else if (op == 1 || op == 2) { local k = keys[rnd(4)]; if (k in t) delete t[k]; }
		else if (op == 3 || op == 4) { local d = rnd(4) == 0 ? null : tables[rnd(tables.len())]; try { t.setdelegate(d); } catch (e) {} }
		else if (op == 5) { if (rnd(2) == 0) { if (t.rawin("_get")) delete t._get; } else t._get <- getters[rnd(getters.len())]; }
		else if (op == 6) { tables[rnd(tables.len())] = rnd(2) ? {} : { a = "new a" + step }; }
		else if (op == 7) { local k = keys[rnd(4)]; ::getroottable()[k] <- "root " + k + step; }
		else { local k = keys[rnd(5)]; out.push(look(t, k)); out.push(look(t, k)); }
	}
	::__out <- out.reduce(@(a, b) a + "\n" + b);
	"#;
	let run = |cache: bool| {
		let vm = SQVM::new(1024);
		vm.set_lookup_cache(cache);
		vm.run_script(SCRIPT, "random").expect("the script runs");
		crate::SqTable::root(&vm).get::<String>("__out").expect("its log")
	};
	let with = run(true);
	let without = run(false);
	assert!(with.lines().count() > 1000, "too few lookups: {}", with.lines().count());
	assert!(with.contains("mm "), "no `_get` answered");
	assert_eq!(with, without);
}
