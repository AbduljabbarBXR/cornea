# CORNEA — Deterministic Visual Inspection for AI Agents

**Codename:** Cornea
**Type:** Lightweight, AI-native layout/visual inspection engine + MCP server that lets a coding agent "see" a page it built — without a heavy browser or pixel-dumping into context.
**Core thesis:** An AI agent doesn't need a *photo* of a page; it needs a *token-efficient, deterministic structural (visual) model* it can reason over exactly. Playwright/Chromium give raw pixels + DOM; that's the wrong interface for an agent's context budget.

---

## 0. Landscape Research (2026) — The Token Wars Are Here

Focused research confirms this is an active, crowded, fast-moving space. Cornea's wedge must be precise.

### What exists right now

| Tool | Approach | Token efficiency | Notes |
|------|----------|------------------|-------|
| **Playwright MCP** | Full Chromium; returns a11y snapshot + screenshot into context | **Poor** (~114k tokens / 10-step task; ~13.7k just for tool defs per request) | The baseline everyone is fleeing |
| **Playwright CLI** (Microsoft, 2026) | Same engine, but **snapshots & screenshots saved to disk**, agent reads what it needs | ~4× better than MCP (~27k / 10 steps) | Key idea: **keep data out of context, let agent opt-in** |
| **agent-browser** (Vercel) | Rust CLI + Node daemon; **a11y tree + screenshot hybrid**; `-i` interactive filter | **Very high** — represents a page in **200–400 tokens** | The current token-efficiency leader |
| **Claude in Chrome** (Anthropic) | Chrome extension + MCP, a11y + screenshot | Medium-low; slow; chrome-extension:// bugs | Shares real login sessions |
| **Lightpanda** (Zig) | Headless browser, **no graphical rendering**, DOM + V8 + CDP | — (no vision) | "Browser for machines"; 9× faster, 16× less memory |
| **AginxBrowser** | Own V8 + **Dioxus Blitz** (Stylo CSS + Taffy layout + vello_cpu raster) for screenshots; single binary, zero Chromium | — | Proves **"feed post-JS DOM to a pure-Rust layout/paint stack"** works |
| **marknative** | Markdown→PNG/SVG, **no browser/DOM**, deterministic layout | — | Proves deterministic layout engines produce pixel-identical output across machines |

### The confirmed lessons (what Cornea must heed)

1. **Keep bulky data OUT of the agent's context; let it opt-in.** Playwright CLI got 4× better by saving to disk. agent-browser won by returning 200–400 tokens.
2. **Accessibility tree >> full DOM/screenshot** for *interaction* — deterministic refs, ~200–400 tokens.
3. **A full browser is unnecessary for "seeing" static layout** — raw HTML/CSS layout is pure math. AginxBrowser proves you can hand layout/paint to a Rust stack and skip Chromium entirely.
4. **The space is converging on "lightweight browser for machines."** Lightpanda and AginxBrowser are already there for *execution/rendering*.
5. **Nobody owns the "deterministic visual INSPECTION / quality reasoning" layer as native MCP tools** — overlap/overflow/contrast/regression as first-class function calls. That is Cornea's open wedge.

---

## 1. Cornea's Positioning (the wedge)

Cornea is **not** another lightweight browser. Lightpanda/AginxBrowser solve *"run a page without Chromium."* Cornea solves:
> **"Give the agent a compact, deterministic description of how the page actually looks and whether it's broken — as direct tool calls, not pixels or raw DOM."**

The differentiator vs. **every** existing tool is a **native visual-model interface**:
- `layout.overlaps` → which elements collide
- `layout.overflow` → what's clipped/collapsed/off-screen
- `layout.contrast` → WCAG ratios
- `layout.compare` → geometry-level regression (no screenshots)
- `layout.responsive` → breakage across viewports
- `layout.at-point(x,y)` → what's under a coordinate

No tool offers these as deterministic, token-cheap, native MCP calls. They all dump pixels and make the *model* figure it out.

---

## 2. Architecture

```
[HTML + CSS (+optional JS)]  
        |
        v
+-------------------------------+
|  DOM Parser (html5ever)       |
|  + Stylesheet parser (CSS)    |
|  + [optional] JS engine       |
+-------------------------------+
        |
        v
+-------------------------------+
|  DETERMINISTIC LAYOUT CORE    |
|  box model · inline · flex    |
|  · grid · absolute/relative   |
|  · z-index · stacking         |
+-------------------------------+
        |
        v
+-------------------------------+
|   VISUAL GEOMETRY MODEL       |
|  (elements + computed box +   |
|   resolved styles + z-order)  |
+-------------------------------+
        |
        v
+-------------------------------+
|   INSPECTION / ANALYSIS       |
|  overlap · overflow · contrast|
|  · alignment · a11y · resp.   |
+-------------------------------+
        |
        v
+-------------------------------+
|   MCP SERVER (stdio) + CLI    |
|    layout.* tools              |
+-------------------------------+
```

**Deterministic end to end:** same HTML/CSS → byte-identical geometry every run (matches the project's determinism ethos and marknative's proven approach).

---

## 3. The Visual Geometry Model (the core artifact)

For each visible element, Cornea computes:

```
ElementView {
  ref: "e21"                        // stable ref (like a11y, for interaction)
  selector: "main .card:nth(2)"
  role: "button" | "text" | "img" | ...
  box: { x, y, w, h }              // viewport space
  contentBox: { x, y, w, h }
  z: int                            // stacking order
  visible: bool                     // display/visibility/opacity/off-screen
  overflow: "hidden"|"visible"|"clip"
  computed: { display, position, flex, grid, margin, padding, ... }
  children: [refs]
  parent: ref
  accessibleName: "..."             // for a11y
  colors: { fg, bg }                // for contrast
}
```

Then **derived conclusions** (high-signal, token-cheap):

| Conclusion | Detection |
|-----------|-----------|
| **Overlap** | two visible sibling/non-sibling boxes intersect in viewport space with neither intentionally stacking over the other |
| **Overflow** | content box exceeds parent clip / viewport bounds |
| **Collapsed** | zero-sized element that should have content |
| **Off-screen** | element fully outside viewport (negative x, beyond right/bottom) |
| **Contrast fail** | computed fg/bg ratio < WCAG AA (4.5:1 text, 3:1 large/UI) |
| **Misalignment** | element boundaries off a shared alignment grid (rhythm check) |
| **Responsive break** | at some viewport width, an element overflows or hides unexpectedly |

---

## 4. Tool Contract (the MCP surface — the wedge)

All tools return **compact JSON**, measured in tokens, designed for agent reasoning:

| Tool | Purpose | Returns (~tokens) |
|------|---------|-------------------|
| `layout.inspect` | Full visual model summary | ~200–600 (configurable depth) |
| `layout.overlaps` | Conflicting elements | ~30–100 |
| `layout.overflow` | Clipped/collapsed/off-screen | ~30–100 |
| `layout.contrast` | WCAG ratios per text el | ~40–120 |
| `layout.at-point(x,y)` | Element under coordinate | ~15 |
| `layout.selectors(sel)` | Resolved geometry for selector | ~20–80 |
| `layout.compare(a.html,b.html)` | Geometry-level visual regression | ~30–150 |
| `layout.responsive(widths)` | Breakage at each width | ~50–200 |
| `layout.quality` | Combined health report | ~100–300 |
| `layout.action` (future) | click/fill via ref | ~10 |

Token budget comparable to or better than **agent-browser's 200–400/page**, but with *inspection* semantics no one else ships.

---

## 5. Deterministic Layout Engine — Feasibility & Scope

This is the hardest part. Options reviewed via research:

| Option | Fidelity | Cost | Verdict |
|--------|----------|------|---------|
| **Full WebKit/Chromium** | 100% | Huge |  defeats the purpose (Playwright is this) |
| **Servo/webrender** | High | Large, complex |  possible later, heavy now |
| **Dioxus Blitz** (Stylo+Taffy+vello_cpu) | High (modern CSS) | Medium; beta |  strong candidate — AginxBrowser proves it; gives real layout + optional CPU raster. **Reuse or learn from Blitz.** |
| **Own constraint engine** | Approx (documented) | Lowest, full control |  recommended starting point for the *inspection* subset (block/inline/flex/position) |

**Recommendation:** Start with a **purpose-built constrained layout core** covering the high-value subset, and treat **Blitz/Taffy** as an integration path to raise fidelity later. Clearly document where layout is approximated (like AginxBrowser does).

**Scoped v1 layout subset (documented as approximate where needed):**
- Block & inline layout, box model, margin/padding/border
- `display: flex` (most common modern layout) — full wrap/grow/shrink/order/align
- `position: static/relative/absolute/fixed/sticky` + z-index + stacking context
- `box-sizing`, `min/max-width/height`, `overflow`
- Basic `display: grid` (v1.5; defer matrix complexity)
- Media queries (responsive) — parse + conditional rules
- Colors + transparency for contrast; text via simple measured widths (real shaping optional via `parley`/`rustybuzz`)

**Deferred:** advanced grid features, sub-pixel shaping precision, complex floats, exotic stacking. Document each as a known approximation.

---

## 6. JS Execution Tier (second decision)

Two tiers (see lessons from AginxBrowser & Lightpanda):

- **Tier 1 (v1, no JS):** Pure layout of static HTML/CSS. Fast, deterministic, in-process, zero engine. Covers most "is my layout broken" questions for the pages agents actually build (which are usually static-ish).
- **Tier 2 (v1.5):** Embed a lightweight JS engine (`boa` or `quickjs`) to run scripts → post-JS DOM → feed to layout. Optionally pair with **AginxBrowser's proven pattern**: run JS elsewhere (own engine or a real browser via CDP), serialize the **final DOM**, hand it to Cornea purely for layout/inspection. This keeps Cornea deterministic and avoids reimplementing browser APIs.

**Recommendation:** Ship Tier 1 first. Adopt the "feed post-JS DOM" pattern for Tier 2 (don't build browser APIs yourself).

---

## 7. Edge Opportunities

1. **Visual regression in CI for $0** — geometry-level compare (`layout.compare`) is deterministic, ~100 tokens, no screenshots. Fits into any agent/CI.
2. **The "eyes" for self-verifying agents** — an agent that calls `layout.overlaps` plus `layout.contrast` after each edit closes its own feedback loop (the AI-sees-its-own-page problem this whole project is about).
3. **Token-truthful page summaries** — `layout.inspect`/`layout.quality` let an agent "read" a page in ~200 tokens vs. MBs of pixels. Compounds over long sessions (the agent-browser/Playwright numbers prove compounding value).
4. **Pair with Crayon** — deterministic geometry → render diagrams/icons/thumbnails; both share the "math over pixels" philosophy and MCP-over-stdio design.
5. **Accessibility as a first-class product** — only Cornea gives WCAG contrast + structure as deterministic tool calls; a11y consultation for agents and humans.
6. **Headless-page-to-vector thumbnails** — reuse geometry model + a vello_cpu/tiny-skia raster for *optional* small human-preview PNGs, without a browser.
7. **Browser-extremity / edge-case fuzzing** — adversarial CSS (brace storms, extreme nesting) is itself a test surface (mirrors heides hostility suite).
8. **Multi-format** — inspect not just HTML but rendered markdown (reuse marknative-style deterministic layouts).

---

## 8. Issues / Setbacks / What the Existing Tools Miss

### What the others miss that Cornea captures
- **Playwright/Chromium:** output is huge, non-deterministic sub-pixel, and requires the model to interpret raw pixels/DOM. No native overlap/contrast/regression reasoning.
- **agent-browser:** best-in-class tokens but for *interaction*, not *visual inspection/quality*. Doesn't answer "is it broken visually."
- **Playwright CLI:** better tokens but still screenshot/DOM-centric; no inspection semantics as native calls.
- **Lightpanda:** no vision/render at all (execution only).
- **AginxBrowser:** gets screenshots without Chromium, but outputs pixels (not a token-efficient visual model), beta quality.
- **CSS-regression tools (backstopjs etc.):** screenshot-diff only; not deterministic geometry; not MCP/agent-native; no overlap/contrast.

### Cornea's genuine setbacks / honest risks
1. **Layout-engine fidelity** is the make-or-break. Flex/grid/stacking are genuinely hard; approximations can yield *wrong* geometry, which is worse than no answer. **Mitigation:** document scope, add a "fidelity trust" field on every result, expose a `layout.fidelity` tool reporting which features are approximated.
2. **"Wrong but confident" danger** — an engine that returns plausible-but-incorrect icon boxes could mislead an agent. **Mitigation:** only compute what the subset can prove; otherwise return `unknown`.
3. **Crowded space tempo** — Playwright/agent-browser move fast; must differentiate on inspection semantics or be commoditized.
4. **JS tier scope creep** — resisting the urge to build browser APIs. **Stick to the "final-DOM-in" pattern.**
5. **Text metrics / fonts** — real text width needs shaping (parley/rustybuzz) or font metrics; without them, inline text layout is approximate. Scope it honestly.
6. **No vision-model dependency** — pure correctness of purpose: Cornea must *never* require a vision model for inspection (deterministic geometry instead). Optional vision only for exotic pixel-only cases.

### What Cornea MUST have to be worth using (the must-haves)
-  **Deterministic, byte-identical geometry** (no sub-pixel variance).
-  **Token-cheap representation** — page reads in 100s of tokens, not MBs.
-  **Native inspection tools** (overlap/overflow/contrast/compare/responsive) — the wedge no one has.
-  **`fidelity` honesty** — every result states its confidence/approx level.
-  **No Chromium dependency**; single binary (Rust) or stdio server.
-  **Opt-in data** (never auto-dump into context; save to disk, agent reads on demand — Playwright CLI lesson).
-  **Stable refs** for interaction (a11y-style) when agent needs to act, not just look.
-  **MCP-over-stdio** + CLI, mirroring heides (no daemon, no ports).
-  **Regression/compare as first-class** — the only cheap deterministic visual-diff.

---

## 9. Tech Stack Recommendation

- **Language:** Rust (determinism, single-binary, speed; ecosystem parity with heides). TypeScript possible but weaker for layout math.
- **DOM parse:** `html5ever` (servo).
- **CSS parse:** `cssparser`/`lightningcss`.
- **Layout:** own constrained core first; **integrate/learn from Taffy + Dioxus Blitz** for higher fidelity. Optional `vello_cpu` (pure CPU raster, no GPU — AginxBrowser proof) for optional human preview.
- **Text shaping (optional):** `rustybuzz`/`parley` for accurate inline width.
- **Grid:** `grid-rs` (implemented by Dioxus/Taffy ecosystem) if adopted.
- **JS (Tier 2):** `boa` or `quickjs` (embed) OR external-parse of final DOM.
- **MCP:** heides-style stdio server, raw-bytes reading, message-size caps, hostile-client tolerant.

---

## 10. Milestones

- **M1 — PoC:** html5ever + box-model layout; `layout.inspect` for a static page (use the sample page from the earlier test); measure tokens vs. screenshot.
- **M2 — Flex + positioning:** flex layout, absolute/relative + z-index; `layout.overlaps`, `layout.at-point`, `layout.selectors`.
- **M3 — Quality:** overflow/collapsed/off-screen; `layout.quality`.
- **M4 — Responsive + media queries:** `layout.responsive`; basic grid (or defer).
- **M5 — Contrast:** WCAG ratio per text el; `layout.contrast`.
- **M6 — Compare:** `layout.compare` geometry regression; version-free deterministic diff.
- **M7 — MCP/CLI polish + fidelity:** `layout.fidelity`; save-to-disk + opt-in reads; OpenCode/Claude/Codex integration; README + token benchmark vs Playwright.
- **M8 — Tier 2 JS (optional):** final-DOM-in pattern for scripted pages.
- **M9 — Hardening:** battle suite, adversarial CSS hostility tests, large-page scale budget, determinism test (same input → byte-identical).

---

## 11. Reference & Follow-Up Research

- **agent-browser (Vercel):** token-efficiency leader (200–400 tokens/page). Study its `-i` interactive filter and concise-output design.
- **Playwright CLI (Microsoft, 2026):** the "save to disk, opt-in read" token strategy; ref-IDs + grep workflow. Directly informs Cornea's data-egress design.
- **AginxBrowser (GitHub, 2026):** "single binary, zero Chromium, feed post-JS DOM to Dioxus Blitz (Stylo+Taffy+vello_cpu)." Best proof Cornea's approach is sound; borrow the integration pattern and honesty about approximations.
- **Dioxus Blitz / Taffy / Stylo:** the layout/paint stack to reuse or learn from for high-fidelity flex/grid.
- **Lightpanda (Zig, 2026):** the "no rendering" execution model; benchmark reference (9× faster, 16× less memory); confirms machines-don't-need-pixels for many tasks.
- **marknative:** deterministic layout proving byte-identical output across machines; reinforces the determinism promise.
- **Playwright MCP Vision Mode / boxes:** shows even Playwright is moving toward giving agents geometry (`boxes` option appends bounding boxes to a11y snapshots) — evidence the industry is converging toward Cornea, i.e., be fast.

**Read next:** benchmark `layout.inspect` token cost against agent-browser and Playwright CLI on a real page; decide reuse-vs-own for the layout engine (Taffy/Blitz) after a PoC; survey whether any determinism-first layout inspection library already exists (likely gap = opportunity).
