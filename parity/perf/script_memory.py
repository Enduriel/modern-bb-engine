#!/usr/bin/env python3
"""Where the script objects' memory goes, from a census of the Squirrel VM's
heap (`SQVM::write_census`, written by the `script_memory` example):

    cd modern_bb_engine
    cargo run --release -p bb-engine --example script_memory -- --mods \\
        --saves DIR --save NAME --census census.txt
    python3 ../parity/perf/script_memory.py census.txt

It splits the heap by object kind and by role in the BB class layout: instance level
tables, their member tables `m` (and what those hold), the inherited methods bound to each
instance, the class prototypes (and each class's own copies of its ancestors' levels), and
everything else; by class; and what the same objects would take in the original's 32-bit
build. Sizes are what the VM asks the allocator for; "chunk" adds glibc's per-allocation
header and 16-byte rounding.
"""

import sys
from collections import Counter, defaultdict


def chunk(n):
    """glibc's chunk for a request of n bytes (64-bit)."""
    return max(32, (n + 8 + 15) & ~15)


def chunk32(n):
    """glibc's chunk in a 32-bit process (MSVC's heap is similar: 8-byte granules)."""
    return max(16, (n + 4 + 7) & ~7)


class Table:
    __slots__ = ("id", "bytes", "nodes", "used", "delegate", "weak", "slots", "flags", "gets", "writes", "lazy", "iterated_at", "cloned_at")

    def __init__(self, f):
        self.id, b, n, u, self.delegate, w = f
        self.bytes, self.nodes, self.used, self.weak = int(b), int(n), int(u), int(w)
        self.slots = []
        self.flags = 0
        self.lazy = None
        self.gets = None
        self.writes = None
        self.iterated_at = None
        self.cloned_at = None


def read(path):
    tables, closures, natives, protos, arrays, other = {}, {}, {}, {}, {}, Counter()
    strings = None
    cur = None
    with open(path, encoding="latin-1") as f:
        for line in f:
            kind = line[0]
            if kind == "s":
                _, k, v = line.rstrip("\n").split(" ", 2)
                cur.slots.append((k, v))
                continue
            if kind == "g" or kind == "w":
                k = line[2:].rstrip("\n")
                if kind == "g":
                    cur.gets = cur.gets or set()
                    cur.gets.add(k)
                else:
                    cur.writes = cur.writes or set()
                    cur.writes.add(k)
                continue
            if kind == "L":
                p = line.split()
                cur.lazy = (p[1], p[2], int(p[3]))
                continue
            if kind == "x":
                cur.flags = int(line[2:])
                continue
            if kind == "r":
                cur.iterated_at = line[2:].rstrip("\n")
                continue
            if kind == "q":
                cur.cloned_at = line[2:].rstrip("\n")
                continue
            p = line.split()
            if kind == "T":
                cur = Table(p[1:])
                tables[cur.id] = cur
            elif kind == "C":
                # id bytes proto env nouters ndefparams weak fetched
                closures[p[1]] = (int(p[2]), p[3], p[4], int(p[5]), int(p[6]), int(p[7]), int(p[8]) if len(p) > 8 else 0)
            elif kind == "N":
                # id bytes env nouters weak fetched name
                natives[p[1]] = (int(p[2]), p[3], int(p[4]), int(p[5]), int(p[6]), p[7] if len(p) > 7 else "")
            elif kind == "F":
                # id bytes ninstr nlit nparams nfuncs nouters nlineinfos nlocalvars ndefparams name source
                protos[p[1]] = (int(p[2]), *map(int, p[3:11]), p[11], p[12] if len(p) > 12 else "")
            elif kind == "A":
                arrays[p[1]] = (int(p[2]), int(p[3]), int(p[4]), int(p[5]))
            elif kind == "S":
                strings = (int(p[1]), int(p[2]))
            elif kind in "OUIKX":
                other[kind] += int(p[2]) if kind != "X" else 0
                other[kind + "#"] += 1
                if kind in "OUIK" and p[3] == "1":
                    other["weak"] += 1
    return tables, closures, natives, protos, arrays, other, strings


def mb(b):
    return f"{b / 1048576:8.1f}"


def ref(v):
    """The object id a slot value refers to, with its kind letter, or None."""
    if v and v[0] in "TACNW" and v[1:3] == "0x":
        return v[0], v[1:]
    return None


def main():
    path = sys.argv[1]
    tables, closures, natives, protos, arrays, other, strings = read(path)

    # Object sizes as allocated (a table is its header plus its node array).
    table_hdr = 88
    t_bytes = sum(t.bytes for t in tables.values())
    t_chunk = sum(chunk(table_hdr) + chunk(t.bytes - table_hdr) for t in tables.values())
    c_bytes = sum(c[0] for c in closures.values())
    c_chunk = sum(chunk(c[0]) for c in closures.values())
    n_bytes = sum(n[0] for n in natives.values())
    n_chunk = sum(chunk(n[0]) for n in natives.values())
    p_bytes = sum(p[0] for p in protos.values())
    p_chunk = sum(chunk(p[0]) for p in protos.values())
    a_bytes = sum(a[0] for a in arrays.values())
    a_chunk = sum(chunk(72) + (chunk(a[0] - 72) if a[0] > 72 else 0) for a in arrays.values())
    weak = sum(t.weak for t in tables.values()) + sum(c[5] for c in closures.values()) + other["weak"]
    w_chunk = weak * chunk(40)
    s_count, s_bytes = strings
    print(f"# Census {path}\n")
    print("| kind | count | MB requested | MB with malloc chunks |")
    print("|---|---:|---:|---:|")
    rows = [
        ("tables", len(tables), t_bytes, t_chunk),
        ("  of which node arrays", sum(t.nodes for t in tables.values()), t_bytes - table_hdr * len(tables), None),
        ("closures (script)", len(closures), c_bytes, c_chunk),
        ("native closures", len(natives), n_bytes, n_chunk),
        ("function prototypes", len(protos), p_bytes, p_chunk),
        ("arrays", len(arrays), a_bytes, a_chunk),
        ("weak references", weak, weak * 40, w_chunk),
        ("strings", s_count, s_bytes, None),
        ("outers", other["O#"], other["O"], other["O#"] * chunk(88)),
        ("userdata", other["U#"], other["U"], None),
        ("native instances", other["I#"], other["I"], None),
    ]
    total = total_chunk = 0
    for name, count, b, c in rows:
        if not name.startswith("  "):
            total += b
            total_chunk += c if c is not None else b
        print(f"| {name} | {count:,} | {mb(b)} | {mb(c) if c is not None else ''} |")
    print(f"| **total** | | {mb(total)} | {mb(total_chunk)} |\n")

    lazy = [t for t in tables.values() if t.lazy]
    if lazy:
        print(f"Lazy tables: {len(lazy):,}, {mb(sum(t.bytes for t in lazy)).strip()} MB, holding {sum(t.used for t in lazy):,} slots of their own.\n")
    used = sum(t.used for t in tables.values())
    nodes = sum(t.nodes for t in tables.values())
    print(f"Table load: {used:,} slots in {nodes:,} nodes ({100 * used / nodes:.0f}%).\n")

    # --- The class layout -------------------------------------------------------------
    leaves = [t for t in tables.values() if any(k == "s_release_hook_DO_NOT_delete_it_" for k, _ in t.slots)]
    role = {}  # table id -> role
    level_of = {}  # level table id -> (leaf id, depth)
    m_of = {}  # m table id -> (leaf id, depth)
    per_class = defaultdict(lambda: Counter())
    bound = set()
    bound_n = set()

    def slot(t, key):
        """A slot's value, read through a lazy table's source as the VM does."""
        while t is not None:
            for k, v in t.slots:
                if k == "s" + key:
                    return v
            t = tables.get(t.lazy[0]) if t.lazy else None
        return None

    for leaf in leaves:
        cls = (slot(leaf, "ClassName") or "s?")[1:]
        depth = 0
        cur = leaf
        while cur is not None:
            role[cur.id] = "level"
            level_of[cur.id] = (leaf.id, depth)
            m = slot(cur, "m")
            if m and m[0] == "T" and m[1:] in tables:
                m_of[m[1:]] = (leaf.id, depth)
            for _, v in cur.slots:
                r = ref(v)
                if r and r[0] == "C" and r[1] in closures and closures[r[1]][2] == leaf.id:
                    bound.add(r[1])
                elif r and r[0] == "N" and r[1] in natives and natives[r[1]][1] == leaf.id:
                    bound_n.add(r[1])
            cur = tables.get(cur.delegate)
            depth += 1
        per_class[cls]["instances"] += 1
        per_class[cls]["levels"] += depth

    # What an instance's `m` tables hold: nested tables and arrays (deep-cloned per instance).
    m_owned_t, m_owned_a = set(), set()
    stack = list(m_of)
    while stack:
        tid = stack.pop()
        for _, v in tables[tid].slots:
            r = ref(v)
            if not r:
                continue
            if r[0] == "T" and r[1] in tables and r[1] not in m_of and r[1] not in role and r[1] not in m_owned_t:
                m_owned_t.add(r[1])
                stack.append(r[1])
            elif r[0] == "A" and r[1] in arrays:
                m_owned_a.add(r[1])

    # Class prototypes and their ancestor copies: tables with ClassNameHash that aren't an
    # instance's level, and their `m` tables.
    proto_levels = [t for t in tables.values() if t.id not in role and slot(t, "ClassNameHash")]
    proto_m = set()
    for t in proto_levels:
        m = slot(t, "m")
        if m and m[0] == "T":
            proto_m.add(m[1:])

    def tbytes(ids):
        return sum(tables[i].bytes for i in ids), sum(chunk(table_hdr) + chunk(tables[i].bytes - table_hdr) for i in ids)

    def cbytes(ids):
        return sum(closures[i][0] for i in ids), sum(chunk(closures[i][0]) for i in ids)

    lv = tbytes(level_of)
    ms = tbytes(m_of)
    mo = tbytes(m_owned_t)
    moa = (sum(arrays[i][0] for i in m_owned_a), sum(chunk(72) + chunk(max(arrays[i][0] - 72, 0)) for i in m_owned_a))
    bc = cbytes(bound)
    bn = (sum(natives[i][0] for i in bound_n), sum(chunk(natives[i][0]) for i in bound_n))
    pl = tbytes([t.id for t in proto_levels])
    pm = tbytes(proto_m)
    leaf_weak = sum(1 for leaf in leaves if leaf.weak)
    print(f"## The class layout: {len(leaves):,} instances, {len(level_of):,} level tables\n")
    print("| part | count | MB requested | MB chunks |")
    print("|---|---:|---:|---:|")
    for name, count, (b, c) in [
        ("instance level tables", len(level_of), lv),
        ("instance `m` tables", len(m_of), ms),
        ("tables held in `m` (deep-cloned)", len(m_owned_t), mo),
        ("arrays held in `m` (deep-cloned)", len(m_owned_a), moa),
        ("inherited methods bound per instance (script)", len(bound), bc),
        ("inherited natives bound per instance", len(bound_n), bn),
        ("leaf weak references (the bound methods' env)", leaf_weak, (leaf_weak * 40, leaf_weak * chunk(40))),
        ("class prototype levels (incl. each class's ancestor copies)", len(proto_levels), pl),
        ("their `m` tables", len(proto_m), pm),
    ]:
        print(f"| {name} | {count:,} | {mb(b)} | {mb(c)} |")
    rest_t = [t for t in tables.values() if t.id not in role and t.id not in m_of and t.id not in m_owned_t
              and t.id not in proto_m and not slot(t, "ClassNameHash")]
    rb = tbytes([t.id for t in rest_t])
    other_cl = [i for i in closures if i not in bound]
    ob = cbytes(other_cl)
    print(f"| other tables | {len(rest_t):,} | {mb(rb[0])} | {mb(rb[1])} |")
    print(f"| other closures | {len(other_cl):,} | {mb(ob[0])} | {mb(ob[1])} |\n")

    # Bound closures: with outers (Modern Hooks/MSU wrappers capture `__original`) or not.
    outer_hist = Counter()
    by_src = Counter()
    by_src_n = Counter()
    for i in bound:
        b, proto, env, nout, ndef = closures[i][:5]
        outer_hist[nout] += 1
        src = protos[proto][10] if proto in protos else "?"
        owner = src[1:].split("/")[0] if src.startswith("s") else src
        by_src[owner] += chunk(b)
        by_src_n[owner] += 1
    print("Bound closures by outer count: " + ", ".join(f"{k}: {v:,}" for k, v in sorted(outer_hist.items())) + "\n")
    print("Bound closures by the source of their function (top 15):\n")
    print("| source | count | MB chunks |")
    print("|---|---:|---:|")
    for owner, b in by_src.most_common(15):
        print(f"| {owner} | {by_src_n[owner]:,} | {mb(b)} |")
    print()

    # Per class (the leaf's class), what one instance costs.
    cls_bytes = defaultdict(Counter)
    for tid, (leaf, depth) in level_of.items():
        cls = (slot(tables[leaf], "ClassName") or "s?")[1:]
        cls_bytes[cls]["levels"] += chunk(table_hdr) + chunk(tables[tid].bytes - table_hdr)
    for tid, (leaf, depth) in m_of.items():
        cls = (slot(tables[leaf], "ClassName") or "s?")[1:]
        cls_bytes[cls]["m"] += chunk(table_hdr) + chunk(tables[tid].bytes - table_hdr)
    for i in bound:
        leaf = closures[i][2]
        cls = (slot(tables[leaf], "ClassName") or "s?")[1:]
        cls_bytes[cls]["bound"] += chunk(closures[i][0])
        cls_bytes[cls]["nbound"] += 1
    ranked = sorted(cls_bytes.items(), key=lambda kv: -(kv[1]["levels"] + kv[1]["m"] + kv[1]["bound"]))
    print("By class (top 25; chunks, MB, and per instance in KB):\n")
    print("| class | instances | levels | MB | level tables | m tables | bound methods | per instance KB | bound per instance |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---:|")
    for cls, c in ranked[:25]:
        n = per_class[cls]["instances"]
        tot = c["levels"] + c["m"] + c["bound"]
        print(f"| {cls} | {n:,} | {per_class[cls]['levels'] / n:.1f} | {mb(tot)} | {mb(c['levels'])} | {mb(c['m'])} | {mb(c['bound'])} | {tot / n / 1024:.1f} | {c['nbound'] / n:.0f} |")
    print()

    tracked = any(t.gets or t.writes or t.flags for t in tables.values())
    if tracked:
        observed(tables, closures, natives, level_of, m_of, bound, bound_n, slot)

    # Function prototypes: what they're made of.
    parts = Counter()
    for p in protos.values():
        b, ni, nl, npar, nf, no, nli, nlv, nd, name, src = p
        parts["instructions"] += ni * 8
        parts["literals"] += nl * 16
        parts["parameters"] += npar * 16
        parts["functions"] += nf * 16
        parts["outer var infos"] += no * 40
        parts["line infos"] += nli * 16
        parts["local var infos"] += nlv * 40
        parts["header"] += 232
    print("Function prototypes by part (MB, approximate): " + ", ".join(f"{k} {v / 1048576:.1f}" for k, v in parts.most_common()) + "\n")

    # The original's 32-bit build: 8-byte values, 4-byte pointers.
    t32 = sum(chunk32(44) + chunk32(t.nodes * 20) for t in tables.values())
    c32 = sum(chunk32(48 + 8 * (c[3] + c[4])) for c in closures.values())
    n32 = sum(chunk32(64 + 8 * n[2]) for n in natives.values())
    a32 = sum(chunk32(36) + (chunk32(a[3] * 8) if a[3] else 0) for a in arrays.values())
    print("The same objects in a 32-bit build (estimate, MB): "
          f"tables {mb(t32).strip()} (64-bit {mb(t_chunk).strip()}), closures {mb(c32).strip()} (64-bit {mb(c_chunk).strip()}), "
          f"native closures {mb(n32).strip()} (64-bit {mb(n_chunk).strip()}), arrays {mb(a32).strip()} (64-bit {mb(a_chunk).strip()})\n")


def sites(tables, ids, what):
    """Where the tables `ids` were first iterated and cloned (top sites)."""
    for attr, verb in (("iterated_at", "iterated"), ("cloned_at", "cloned")):
        c = Counter(getattr(tables[i], attr) for i in ids if getattr(tables[i], attr))
        if c:
            print(f"  - {what} first {verb} at: " + "; ".join(f"{k} {n:,}" for k, n in c.most_common(8)))


def observed(tables, closures, natives, level_of, m_of, bound, bound_n, slot):
    """What the scripts touched while tracking was on (the census's g/w/x lines)."""
    print("## What scripts touched while tracked\n")
    fetched = sum(1 for i in bound if closures[i][6])
    fetched_n = sum(1 for i in bound_n if natives[i][4])
    print(f"- Bound inherited methods fetched: {fetched:,} of {len(bound):,} ({100 * fetched / max(len(bound), 1):.1f}%); "
          f"bound natives fetched: {fetched_n:,} of {len(bound_n):,}.")
    anc = [tid for tid, (leaf, d) in level_of.items() if d > 0]
    leaves = [tid for tid, (leaf, d) in level_of.items() if d == 0]
    def count(ids, pred):
        return sum(1 for i in ids if pred(tables[i]))
    print(f"- Ancestor levels ({len(anc):,}): iterated {count(anc, lambda t: t.flags & 1):,}, cloned {count(anc, lambda t: t.flags & 2):,}, "
          f"written {count(anc, lambda t: t.writes):,}, `m` read through them {count(anc, lambda t: t.gets and 'sm' in t.gets):,}, "
          f"any slot read {count(anc, lambda t: t.gets):,}.")
    print(f"- Leaves ({len(leaves):,}): iterated {count(leaves, lambda t: t.flags & 1):,}, cloned {count(leaves, lambda t: t.flags & 2):,}, "
          f"written {count(leaves, lambda t: t.writes):,}.")
    wk = Counter()
    for i in anc:
        for k in tables[i].writes or ():
            wk[k] += 1
    if wk:
        print("  - keys written on ancestor levels: " + ", ".join(f"{k} {n}" for k, n in wk.most_common(12)))
    wk = Counter()
    for i in leaves:
        for k in tables[i].writes or ():
            wk[k] += 1
    if wk:
        print("  - keys written on leaves: " + ", ".join(f"{k} {n}" for k, n in wk.most_common(12)))
    ms = list(m_of)
    written = sum(len(tables[i].writes or ()) for i in ms)
    used = sum(tables[i].used for i in ms)
    read = sum(len(tables[i].gets or ()) for i in ms)
    print(f"- `m` tables ({len(ms):,}, {used:,} fields): iterated {count(ms, lambda t: t.flags & 1):,}, cloned {count(ms, lambda t: t.flags & 2):,}, "
          f"written {count(ms, lambda t: t.writes):,} ({written:,} distinct fields written), {read:,} distinct fields read.")
    per_depth = defaultdict(lambda: [0, 0, 0])
    for i in ms:
        d = m_of[i][1]
        per_depth[d][0] += 1
        per_depth[d][1] += tables[i].used
        per_depth[d][2] += len(tables[i].writes or ())
    print("  - by depth (0 = leaf): " + "; ".join(f"{d}: {n:,} tables, {u:,} fields, {w:,} written" for d, (n, u, w) in sorted(per_depth.items())))
    print(f"  - slots added {count(ms, lambda t: t.flags & 4):,}, removed {count(ms, lambda t: t.flags & 8):,}; "
          f"iterated after that {count(ms, lambda t: t.flags & 16):,}, cloned after that {count(ms, lambda t: t.flags & 32):,}")
    sites(tables, ms, "`m` tables")
    lazy_ms = [i for i in ms if tables[i].lazy]
    if lazy_ms:
        # Lazy members: which `m` tables were made plain, and why.
        plain = [i for i in ms if not tables[i].lazy and tables[i].used]
        def mbytes(ids):
            return sum(chunk(88) + chunk(tables[i].bytes - 88) for i in ids)
        print(f"  - lazy {len(lazy_ms):,} ({mb(mbytes(lazy_ms)).strip()} MB); plain and not empty {len(plain):,} ({mb(mbytes(plain)).strip()} MB), of which:")
        groups = [
            ("iterated, no slot added or removed before", lambda t: t.flags & 1 and not t.flags & 16),
            ("iterated after a slot was added or removed", lambda t: t.flags & 16),
            ("a slot removed, not iterated", lambda t: t.flags & 8 and not t.flags & 1),
            ("untracked (made plain during the load, or never lazy)", lambda t: not t.flags & 9),
        ]
        for label, pred in groups:
            ids = [i for i in plain if pred(tables[i])]
            # What each would hold lazy: its written fields and nested containers.
            def own(t):
                cont = sum(1 for k, v in t.slots if v[:1] in "TA")
                n = len(t.writes or ()) + cont
                size = 4
                while size * 3 < n * 4:
                    size *= 2
                return chunk(88) + chunk(size * 32) + 96
            est = sum(own(tables[i]) for i in ids)
            print(f"    - {label}: {len(ids):,}, {mb(mbytes(ids)).strip()} MB (lazy they'd hold ~{mb(est).strip()} MB)")
    # Fetched per instance.
    per_inst = Counter()
    for i in bound:
        if closures[i][6]:
            per_inst[closures[i][2]] += 1
    n_inst = len(leaves)
    hist = Counter(min(per_inst.get(t, 0) // 5 * 5, 50) for t in leaves)
    print("- Bound methods fetched per instance (bucket: instances): " + ", ".join(f"{k}+: {v:,}" for k, v in sorted(hist.items())) + "\n")


if __name__ == "__main__":
    main()
