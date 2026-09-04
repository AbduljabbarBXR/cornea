# ROADMAP: JavaScript / SPA rendering for Cornea

**Status:** Plan (not yet implemented)
**Goal:** Extend the deterministic static-HTML engine to render pages whose layout comes from **script** (vanilla JS, and — as a later stretch — React/SPA), while preserving Cornea's core promise: **deterministic, in-process, single binary, no Chromium, no network.**

---

## 0. The honest constraint

Cornea today inspects **static HTML** you hand it. Script-driven pages (JS, React, SPAs) render their DOM *at runtime*, which a pure HTML parser cannot see.

Two ways to see rendered DOM:
1. **Headless Chromium** (Playwright) — faithful, but breaks every Cornea promise (huge binary, network, non-deterministic timing, server dependency). **Rejected** for now.
2. **An in-process JS engine rendering against a DOM shim** — keeps all promises, but the engine has **no built-in DOM**. This is the chosen path.

The chosen route means Cornea **implements a minimal DOM in the Rust/CJS layer** that the JS runs against. Full browser parity is *not* the target; a **sufficient** DOM for the layout engine is.

---

## 1. Architecture

```
static HTML ──parse──▶ DOM tree ──[no <script>]──────────────▶ layout engine (existing)
                            │
                            └─[has <script>, --js on]─────▶ boA JS engine
                                                              │  runs inline <script> +
                                                              │  <script src> from embedded bundle
                                                              │  writing to a JS-side DOM shim
                                                              ▼
                                                     rendered DOM (mutated)
                                                              │
                                                              └─▶ layout engine (existing)
```

**Key idea:** Cornea already builds its own `dom::Node` tree from html5ever. For JS we add a **JS-facing DOM** (the "shim") that the script mutates, then **re-flatten** the mutated shim back into Cornea's layout model. The layout/overlap/contrast engine is unchanged — only the DOM *source* gains a script-execution step.

---

## 2. Dependency

- `boa_engine = "0.22"` — pure-Rust/WASM JS engine (lexer, parser, interpreter, ES intrinsics).
- No Chromium, no external browser, no network, no OS process — determinism preserved.
- Binary grows, but stays a single static-ish binary (dependent on how boA is compiled; expect +several MB from ~1.4MB).

---

## 3. The DOM shim (the hard, real work)

boA gives a JS engine only. We must supply the web surface the script expects. Scope the shim explicitly:

**MVP shim (implement now):**
- `document`: `getElementById`, `getElementsByTagName`, `querySelector` (subset), `createElement`, `createTextNode`, `body`
- `element`: `appendChild`, `removeChild`, `insertBefore`, `setAttribute`, `getAttribute`, `style` (object with camelCase properties), `textContent`, `innerHTML` (parsed via our own html parser), `children`, `parentNode`, `className`/`classList`
- `document.documentElement` / `document.body`
- `node`/`element` type stubs

**Deferred (Phase 2 / stretch):**
- `Event`/event listeners, `addEventListener`, `window` timers (`setTimeout`/`requestAnimationFrame`)
- `fetch`/`XMLHttpRequest`, `localStorage`, `location`, `history`
- Real selector engine (combinators, pseudo) — reuse/extend the CSS layer
- React/SPA mounting (see §5)

**Explicitly not in scope:** `canvas`, WebGL, layout queries (`getComputedStyle` feed? — see below), `IntersectionObserver`, media queries, GPU/raster.

---

## 4. Determinism policy (what makes Cornea still "deterministic")

Script execution is the biggest new nondeterminism source. Policy:

- **Sync-only execution.** Run the page's script(s) to completion, blocking, in document order. No event loop, no timers.
- **Timers/network are errors.** `setTimeout`, `fetch`, `XMLHttpRequest`, `Promise` that never settles → Cornea records a **fidelity/behavior note** ("async API used; result may be incomplete") rather than silently hanging. Determinism stays a *guarantee*, documented as: *"same input ⇒ identical output,"* with async APIs intentionally unsupported.
- **No wall-clock, no RNG seed variation.** Any `Date.now`/`Math.random` usage is flagged in the report as a non-determinism source.
- The engine still guarantees byte-identical output for the supported subset.

**Honest framing:** Cornea remains deterministic *for the subset it can execute*. For pages that require async/timers/network to mount, Cornea reports the limitation in `fidelity` rather than faking it.

---

## 5. React / SPA support — honest phase-gating

Full React mounting needs a largely complete DOM + synthetic events + reconciliation hooks. **Do not promise this in the MVP.**

- **Phase A (MVP, this roadmap):** vanilla inline `<script>` + basic DOM manipulation (the shim above). Reach: many static pages that inject layout via JS.
- **Phase B (stretch):** React without JSX compiler issues — requires bundling React's `react-dom` into the embedded bundle, a robust `innerHTML` DOM, and event dispatching. Mark as **experimental**, gated behind `--js=full`, with the fidelity report noting "experimental, subject to breakage."
- **Phase C (explicitly out):** arbitrary npm/webpack SPAs. Realistically needs Chromium; revisit only if the product shifts.

**Recommendation:** Ship Phase A, prove the shim with real fixture pages, **then** evaluate Phase B against a chosen React app. Avoid boilerplate promise of "renders any React app."

---

## 6. Control surface (CLI / MCP / HTTP parity)

New opt-in path, default **unchanged**:

- CLI: `cornea page.html --js` (execute scripts; default off for backward compat)
- MCP: `layout.inspect` gains `js: true` argument (off by default)
- HTTP: `POST /inspect` gains `{"js": true}`
- `layout.fidelity` gains: `"js": {"engine":"boa","dom_shim":"<version>","script_count":N,"unsupported_apis":[...]}` — so consumers see exactly what ran.

All surfaces still route through the **same** `rest::dispatch`, so determinism/parity holds.

---

## 7. Testing strategy

- Unit: shim primitives (`document.getElementById`, `appendChild`, `innerHTML`) against hand-written widget scripts.
- Integration (end-to-end binary): a fixture page that JS-builds an overlapping layout; assert overlaps/overflow detected **after** script runs and **not** before (`--js` off).
- Determinism regression: run the same JS fixture twice → assert byte-identical.
- Unsupported-API test: fixture using `setTimeout`/`fetch` → assert report records the unsupported flag, does not hang.
- Keep all existing 28 tests green (they exercise the static path; `--js` default off must not perturb them).

---

## 8. Concrete next steps (when approved)

1. Add `boa_engine = "0.22"` dep; `cargo check` picks it up (verify network/build).
2. Scaffold `src/js/` module: `shim.rs` (JS heap objects) + `exec.rs` (run scripts over the DOM tree) + `render.rs` (flatten mutated shim → `dom::Node`).
3. Wire the `--js` flag through `main.rs`, `rest.rs`, `server_http.rs` input schema.
4. Land **Phase A** shim + fixtures + the 4 test classes above.
5. Update `layout.fidelity`, `README`, and `PUBLISHING.md`; bump version for the crates.io/registry release.

**Risks:** shim completeness is open-ended (avoid scope creep on `cssom`/layout-query APIs); boA build time/size; React phase realistically hard. Mitigate by keeping `--js` opt-in and Phase-A-scoped.

---

## 9. Do NOT break the core promise

- Default remains static, deterministic, dependency-free layout.
- `--js` is opt-in; when off, behavior is byte-identical to today (guards the 28 tests and existing consumers).
- Any plan that silently compromises determinism or adds a hard runtime dependency (Chromium) is rejected unless the product definition changes.
