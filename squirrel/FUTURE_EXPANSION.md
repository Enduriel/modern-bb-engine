# Squirrel bindings — potential future expansions

A running list of features that are feasible but deliberately out of scope today,
with enough notes to pick them up later.

## Crate module layout

The crate favours many small, narrowly-scoped files over large ones. Current
shape:

```
src/
  consts.rs         SQ_OK / SQ_ERROR
  object.rs         SqObject / SqObjectType — raw HSQOBJECT mirror
  vm/
    mod.rs          SQVM type, lifecycle, as_ptr(), print/compiler-error setup
    stack.rs        typed get_*/push_*/get_str_ref/stack_top/get_type/throw_error
    script.rs       execute_squirrel_script, squirrel_alloc + release_hook
    print.rs        default print/error/compile-error handlers
  function/         the native-function / closure system
    mod.rs          SqFunction trait, trampoline, SQVM::{register, make_closure, push_closure}
    arg.rs          SqArg (GAT) + impls (incl. SqTable 't' / SqArray 'a' / SqRef '.' any-type)
    ret.rs          IntoSqReturn + impls
    this.rs         This<'vm>                    variadic.rs  Variadic<T>
    into_fn.rs      IntoSqFunction, markers, ClosureFn, the per-arity macro
    strong_ref.rs   SqStrongRef (owned ref; Clone/push/as_table, impl SqValue, CallArgs)
    number.rs       Number (lenient f64 arg) + DynNumber (int|float-preserving arg+return)
    tests.rs        registration / This / borrowed-str / table-arg / make_closure tests
  types/            safe wrappers for complex (reference) types
    value.rs        SqValue (GAT) — element conversion (Item<'a>)
    array.rs        SqArray<'vm> — Vec-like live array handle (+ nesting, deep_clone, to_strong)
    table.rs        SqTable<'vm> — map handle (+ call_method, deep_clone/deep_merge_from)
    dynamic.rs      SqRef<'vm> — matchable dynamic value enum
    deep.rs         push_deep_clone — recursive copy backing deep_clone/merge
  class/            native classes (Rust-backed SQ classes / instances)
    payload.rs      NativePayload<T> (TypeId header) + shared typetag + release hook + read-back
    property.rs     FieldValue, Property, GetMeta/SetMeta (the _get/_set metamethods)
    builder.rs      ClassBuilder<T> / SqClass<T> — define a class, instantiate
    arg.rs          Native<T> — take an instance as a native-fn argument
    tests.rs        round-trip / cross-type rejection / release-on-drop / stress
```

The native-class API (`class/`) is the generic mechanism; the concrete BB value
types (`vec2f`/`vec2i`/`color`) are built on it in the `squirrel-bb` crate (the
`squirrel` crate stays native-only). Instances carry a boxed
`NativePayload<T>` via `sq_setinstanceup`, freed by a per-instance release hook;
all native classes share one typetag and `payload_ptr` checks typetag + embedded
`TypeId` before any deref. `get_type` never panics (it degrades an unknown tag to
`Null`) since it runs inside the `extern "C"` trampoline.

Cross-module access to the raw `HSQUIRRELVM` goes through `SQVM::as_ptr()`
(pub(crate)); the `ptr` field stays private to `vm/mod.rs`.

**Complex-type interfacing** lives in the sibling **`types/`** module, one file
per Squirrel type, layered on `object.rs` (raw mirror) + `vm/stack.rs`
(marshalling) + `function/`. `types/value.rs` holds the shared by-copy element
trait `SqValue`. Keep each type in its own file rather than growing `object.rs`.

- **`types/array.rs` — `SqArray` (done, incl. nesting).** A borrowed (`'vm`),
  Vec-like handle over a live Squirrel array: `new`/`len`/`get`/`set`/`push`/`pop`/
  `insert`/`remove`/`clear`/`reverse`/`iter`/`to_vec`, plus `set_global`/
  `from_global` to round-trip with scripts. Holds an addref'd `HSQOBJECT`, releases
  on `Drop`; each op pushes the object, does the work, pops back to a balanced
  stack. Element conversion is the GAT `SqValue` (`types/value.rs`): `get::<i64>`
  yields owned, `get::<SqArray>` yields a nested handle.
- **`types/table.rs` — `SqTable` (done).** Borrowed, map-like string-keyed handle:
  `new`/`root`/`len`/`get`/`get_ref`/`contains`/`set` (upsert)/`remove`/`keys`/
  `entries`/`shallow_clone`/`deep_clone`/`deep_merge_from`/`set_delegate`/`delegate`/
  `call_method`/`to_strong`, plus `impl SqValue` (nestable). Reads **follow the
  delegate chain** (`sq_get`: `DONT_FALL_BACK` is the *selfidx*, so it only disables
  the implicit root-table fallback — delegates ARE followed; verified). `call_method`
  is this-bound + delegate-following (finds inherited `create()`). `SqTable::root(vm)`
  wraps the globals table — the surface the module system + most engine APIs use.
- **`types/dynamic.rs` — `SqRef<'vm>` (done).** A `#[non_exhaustive]`, matchable
  value enum (`Null`/`Integer`/`Float`/`Bool`/`String`/`Array(SqArray<'vm>)`/
  `Table(SqTable<'vm>)`/`Other(SqObjectType)`), via `get_ref`. Carries only safe
  handles, never the raw `SqObject` pointers. `Other` is read-only for now
  (`push_value` → `unimplemented!`); the intent is for every variant to round-trip.
- **Semantics to remember:** nested arrays/tables are **reference-shared, not
  copied** (Squirrel containers are reference types; `shallow_clone`/`Clone` is one
  level). Reads return **one level only** (a child handle), which keeps **cyclic
  containers** (`a.append(a)`) from looping — the caller drives recursion.
  `deep_clone`/`deep_merge_from` (via `deep.rs`) recurse into nested tables/arrays
  and **assume acyclic input** (config / `m`-default data); a future deep op over
  possibly-cyclic data MUST carry a visited-set of container identities.
- **`function/strong_ref.rs` — `SqStrongRef` (done).** The owned/persistent
  complement to the borrowed handles: `'static`, addref'd, releases on `Drop`,
  captured from a native-fn param via `impl SqArg` (mask `c`), with `call`/
  `call_ret`. For the engine to retain SQ **closures** across calls (scheduleEvent,
  tile effects, save/load hooks, onRender, UI handles).
- **`types/class.rs`, `types/instance.rs` — TODO.** Same borrowed-handle pattern;
  reuse `SqValue` + add `Class`/`Instance` variants to `SqRef` (`#[non_exhaustive]`
  makes that non-breaking). Instances also need userdata + a release hook for the
  Properties clone pipeline. An owned/persistent *value* handle (holding the VM via
  `Rc` so it can outlive a borrow / be stored) remains the open cross-cutting
  design question; the borrowed model + `SqStrongRef` covers needs so far.

## Custom methods on built-in types (default-delegate extension)

**Status:** feasible, not implemented.

Every built-in Squirrel type (string, integer/float "number", array, table,
generator, closure, thread, class, instance, weakref) has a single per-VM
**default delegate** — a real `SQTable` of methods, created once at VM init from
the `*_default_delegate_funcz[]` arrays
(`squirrel-sys/squirrel/squirrel/sqstate.cpp` `SQSharedState::Init`). Method calls
on a value route through it via `SQVM::InvokeDefaultDelegate`
(`sqvm.cpp`, `case OT_STRING: ddel = _string_ddel; ...; return ddel->Get(key,dest)`).

That table is the *live, shared* one and is mutable. The C API exposes it
(already in our bindings):

```rust
// pushes ss->_string_default_delegate by reference (NOT a copy)
sq_getdefaultdelegate(vm.ptr, OT_STRING);
```

So you can add a method to a built-in type by fetching its default delegate and
adding a native-closure slot:

```rust
// sketch
sq_getdefaultdelegate(vm.ptr, OT_STRING);   // delegate table on top
vm.push_str("shout");                         // method name
// ... push trampoline closure + userdata ...
sq_newslot(vm.ptr, -3, false);                // installs it
// script:  "hi".shout()   -> calls the closure with `this` == "hi"
```

The receiver is passed as `this` at stack index 1 — confirmed by the built-in
methods themselves, e.g. `{"len", default_delegate_len, 1, "s"}` in
`sqbaselib.cpp` declares typemask `"s"` for the `this` slot. So for these methods
`this` is the *value* (a string / integer / float), not a table or instance.

**Implications / design notes:**

- This is why `This` is worth keeping for value types, not just tables/classes/
  instances. A string/number method genuinely receives the value as `this`.
- It is **VM-global and coarse**: mutating `_string_default_delegate` affects
  every string in the VM, permanently. There is no per-value delegate for value
  types. Script cannot do this itself (only `table.setdelegate` exists,
  `sqbaselib.cpp`); it is a C-API / library-author capability.
- To make value-type receivers actually useful, `This` needs **typed accessors**
  (`as_str() -> Option<&'vm str>`, `as_i64()`, `as_f64()`, `as_bool()`). The clean
  implementation is to have `This` hold the real `&'vm SQVM` instead of just
  `PhantomData`, so the accessors reuse the existing safe `get_string` /
  `get_integer` / … helpers reading index 1. The `'vm` borrow already guarantees
  the value outlives the handle.
- Likely worth a small `register_method(type_tag, name, closure)` helper that
  wraps the `sq_getdefaultdelegate` + `sq_newslot` dance, reusing the same
  `IntoSqFunction` schema derivation as `register`.

## Panic-free trampoline boundary

**Status:** partially done.

`This::extract` is now panic-free (guards the `sq_getstackobj` result and the
`_type` tag before `SqObject::from_raw`). The same panic-across-FFI class still
exists pre-existing in `SQVM::get_type` (`.expect("Invalid SQObjectType")`),
reachable inside the trampoline via `Option<T>::from_sq`. It is only
theoretically triggerable (paramscheck validates args first, and `sq_gettype`
always returns a valid tag), but if we want the trampoline path provably
unwind-free, either make `get_type` non-panicking or wrap the trampoline body in
`catch_unwind`.
