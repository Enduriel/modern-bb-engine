# UI bridge probes

Probes for the ORIGINAL game, run with a probe runner that launches the original game
(not part of this repository; point `BB_PROBE_RUNNER` at it). They established the
Coherent bridge's behavior that `bb-ui` reproduces.

| Probe | Checks |
|---|---|
| `bridge_order.nut` + `.js` | **Inline calls**: `asyncCall`, `call` and `UI.connect` run their JS before returning, and `SQ.call` runs its script before the next JS statement. Also: `callSQ` return values and timing, void calls never resolving, handles, the native `engine` members, `compatMode`, dynamic script order, `coui://` resolution. |
| `bridge_values.nut` + `.js` | **Conversions**: JS→SQ numbers, holes, `undefined`, functions; SQ→JS non-string keys, closures, instances, weakrefs. Also `RootScreen`'s handles, return values, and JS exceptions. |

Run from the repo root (menu mode, hidden and muted; about a minute each), with
`BB_PROBE_RUNNER` naming the probe runner:

```bash
python3 "$BB_PROBE_RUNNER" bb-ui/probes/bridge_order.nut \
    --js bb-ui/probes/bridge_order.js --mode menu --out <out-dir>/bridge_order
```

**Don't** add an SQ array holding a closure or instance to `asyncCall` data: the original
crashes ("Array size mismatch").
