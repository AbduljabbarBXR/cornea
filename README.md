# Cornea 👁️

**Deterministic visual inspection for AI agents — the eyes a coding agent never had.**

Cornea lets an AI agent that builds web pages actually *see* them — as a **token-cheap, deterministic structural model** it can reason over exactly, instead of megabytes of screenshots and raw DOM dumped into context.

An agent doesn't need a *photo* of a page; it needs to know **is my layout broken, and how**. Cornea computes an abstract *visual geometry model* of every element (box, position, z-order, computed styles, overlap, overflow, contrast) and exposes it as native, deterministic inspection tools.

---

## What it does

- **Deterministic layout engine** — computes real geometry (box model, inline, flex, grid, absolute/relative, z-index, stacking) with **byte-identical output** per input. No sub-pixel variance, no Chromium.
- **Native inspection tools (MCP)** — the wedge no other tool offers:
  - `layout.overlaps` — which elements collide
  - `layout.overflow` — clipped / collapsed / off-screen
  - `layout.contrast` — WCAG contrast ratios per text element
  - `layout.compare` — geometry-level visual regression (no screenshots)
  - `layout.responsive` — breakage across viewport widths
  - `layout.inspect` / `layout.at-point` / `layout.selectors`
- **Token-cheap** — reads a page in **hundreds of tokens**, not MBs. Data stays on disk and the agent opts in (the Playwright-CLI lesson).
- **`fidelity` honesty** — every result states how confident/approximate it is, so it never misleads the agent ("wrong but confident" danger).
- **No Chromium dependency** — Rust, single binary, MCP over stdio (heides-style).

## Research foundation (2026 token war)

The space is crowded and moving fast. Cornea's wedge is precise — **nobody owns deterministic visual *inspection* semantics as native agent tools**:

- **agent-browser (Vercel)** — token-efficiency leader (200–400 tokens/page) but for *interaction*, not visual *quality reasoning*.
- **Playwright CLI (Microsoft, 2026)** — 4× better tokens by saving snapshots/screenshots **to disk** and letting the agent opt-in.
- **AginxBrowser** — proves "single binary, zero Chromium: feed post-JS DOM to a pure-Rust layout/paint stack (Dioxus Blitz: Stylo+Taffy+vello_cpu)."
- **Lightpanda (Zig)** — "browser for machines," DOM + V8, no rendering at all.
- **marknative** — proves deterministic layout engines give pixel-identical output across machines.

Cornea is **not** another lightweight browser. It's the deterministic *visual reasoning* layer those browsers hand raw pixels to.

## Positioning

**This is the "visual nervous system" for web pages** — the perceptual counterpart to heides' code nervous system. Playwright/Chromium render pixels; Cornea reasons about whether the page is actually broken, in tokens an agent can afford.

---

## Architecture

```
[HTML + CSS (+optional JS)] -> [Deterministic Layout Core]
        (html5ever / cssparser / flex / grid / z)       |
                                                        v
                                              [Visual Geometry Model]
                                                        |
                                                        v
                                    [Inspection: overlap / overflow / contrast /
                                     alignment / responsive / compare]
                                                        |
                                                        v
                                        [MCP Server (stdio) + CLI]
                                         layout.* tools
```

**Read the full technical spec:** [`CORNEA-ARCHITECTURE.md`](./CORNEA-ARCHITECTURE.md)

---

## Research Execution Plan

### Phase 0 — Validate the thesis (do this first)
- [ ] **Reuse audit** — survey devops repos (`terminull`, `open-mobile`, `cortes`, `omni`) and ecosystem (Dioxus Blitz, Taffy, Lightpanda, AginxBrowser) for reusable layout/rendering code.
- [ ] **PoC token benchmark** — build a tiny box-model layout for a sample static page; measure `layout.inspect` token cost vs. agent-browser and Playwright CLI.
- [ ] **Engine decision** — own minimal layout core vs. adopt/integrate Taffy/Blitz. Decide before committing.

### Phase 1 — MVP (Milestones 1–3)
- [ ] `html5ever` parse + box-model layout for a static page
- [ ] `layout.inspect` tool; compare token cost (heides-style MCP server)
- [ ] Flex + absolute/relative + z-index; `layout.overlaps`, `layout.at-point`, `layout.selectors`
- [ ] Overflow / collapsed / off-screen; `layout.quality`

### Phase 2 — Inspection depth (Milestones 4–6)
- [ ] Responsive + media queries; `layout.responsive`
- [ ] WCAG contrast; `layout.contrast`
- [ ] `layout.compare` — deterministic geometry-level visual regression

### Phase 3 — Agent integration (Milestones 7–9)
- [ ] `layout.fidelity`; save-to-disk + opt-in reads (Playwright-CLI lesson)
- [ ] OpenCode / Claude / Codex integration; README + token benchmark vs Playwright
- [ ] Tier-2 JS: feed **final post-JS DOM** to layout (AginxBrowser pattern), no browser APIs reimplemented
- [ ] Hardening: adversarial CSS hostility suite, scale budget, determinism test

---

## Directory layout (planned)

```
cornea/
├── CORNEA-ARCHITECTURE.md  # full technical spec (this repo)
├── README.md               # this file
├── spec/                   # layout-subset scope + documented approximations
├── src/                    # Rust
│   ├── dom.rs              # html5ever parser
│   ├── css.rs              # stylesheet parser
│   ├── layout.rs           # deterministic layout core (box/flex/grid/z)
│   ├── model.rs            # visual geometry model
│   ├── inspect.rs          # overlap / overflow / contrast / compare
│   └── server.rs           # MCP server (layout.* tools)
├── tests/                  # determinism + hostility suites
└── bench/                  # token benchmarks vs playwright/agent-browser
```

## License

Private research repo. All rights reserved. Not for public distribution until release decision.
