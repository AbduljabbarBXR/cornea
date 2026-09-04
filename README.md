# Cornea 👁️

**Deterministic visual inspection for AI agents — the eyes a coding agent never had.**

Cornea gives a coding agent that builds web pages a real way to *see* them — as a **token-cheap, deterministic structural model** it can reason over exactly, instead of megabytes of screenshots and raw DOM dumped into context.

An agent doesn't need a *photo* of a page. It needs to know **is my layout broken, and how.** Cornea computes an abstract *visual geometry model* of every element (box, position, z-order, computed styles) and derives inspection conclusions — **overlap, overflow, contrast, quality** — then exposes them as native tools on **three surfaces**: CLI, MCP, and HTTP API.

> One promise holds everything together: **same input → byte-identical output.** No Chromium, no sub-pixel variance, no server. A single ~1.4 MB binary.

---

## Quick start

```bash
# Build (Rust 1.98+, edition 2024)
cargo build --release

# Inspect a file — the simplest way to use Cornea
./target/release/cornea tests/fixtures/sample-bugs.html 360
```

> Requires only `cargo` — no browser, no Node, no system dependencies. Builds fine on a phone-class device.

---

## Visual guide: what Cornea does

Take this page below. It looks fine as source — but it's hiding four layout bugs. Cornea finds every one.

```html
<!-- tests/fixtures/sample-bugs.html (excerpt) -->
<section class="row">
  <div class="card">Card 1</div><div class="card">Card 2</div><div class="card">Card 3</div>
</section>
<div class="overlap-left">Left overlap</div>   <!-- position:absolute; left:20; top:20 -->
<div class="overlap-right">Right overlap</div> <!-- position:absolute; left:150; top:40 -->
<div class="overflow-bad">...</div>            <!-- width:600 on a 360 viewport -->
<p class="low-contrast">Hard to read on white</p>  <!-- color:#cccccc on #ffffff -->
```

Run it and Cornea reports the damage instantly:

```text
$ ./target/release/cornea tests/fixtures/sample-bugs.html 360

{
  "html_file": "tests/fixtures/sample-bugs.html",
  "viewport_w": 360.0,
  "element_count": 17,
  "est_tokens": 1068,                      // <-- entire page read for ~1k tokens
  "report": {
    "total_elements": 17,
    "visible_elements": 14,
    "overlaps": [ ... 7 collisions ... ],
    "overflows": [ ... 1 clipped ... ],
    "contrast":  [ ... 2 AA failures ... ],
    "quality":   { "score": 0.06, "label": "broken" }
  }
}
```

Each finding is precise and actionable:

| Finding | Detail |
|--------|--------|
| **Overlap** | `section.row ⇄ div.overlap-right`, area 20000 px² — the absolutely-positioned boxes cover the cards |
| **Overflow** | `div.overflow-bad`: right edge `600` exceeds viewport `360` → clipped |
| **Contrast** | black on `blue`: ratio `2.44:1` — fails WCAG AA (needs 4.5) |
| **Contrast** | `#cccccc` on white: ratio `1.61:1` — fails WCAG AA |

That is the value: **hundreds of tokens, not hundreds of kilobytes**, and a deterministic answer the agent can act on and re-verify.

---

## Visual guide: the three surfaces

Cornea is **one engine, three doors.** All three returns identical inspection JSON because they funnel through a single shared dispatch.

### 1. CLI — inspect a file

```bash
cornea <file.html> [viewport_width]
```

```text
$ cornea page.html 360
{
  "html_file": "page.html",
  "viewport_w": 360.0,
  "element_count": 42,
  "json_bytes": 8124,
  "est_tokens": 2193,
  "report": { "total_elements": 42, "visible_elements": 38, "overlaps": [], "overflows": [], "contrast": [], "quality": { "score": 1.0, "label": "good" } }
}
```

### 2. MCP — native agent tools over stdio

```bash
cornea --serve
```

An agent calls `layout.*` tools directly; the page source is passed per-call and stays on disk:

```json
→ {"method":"tools/call","params":{"name":"layout.overlaps",
     "arguments":{"html":"<div style=\"position:absolute;left:20;top:20;width:200;height:100\">A</div>..."
                   ,"width":360}}}
← {"id":1,"result":{"content":[{"text":"[{\"a_sel\":\"...div \u21c4 ...div\",\"area\":20000}]"}]}}
```

| Tool | Returns |
|------|---------|
| `layout.inspect` | Full visual model + report |
| `layout.overlaps` | Elements whose boxes collide |
| `layout.overflow` | Clipped / collapsed / off-screen |
| `layout.contrast` | WCAG AA ratios for text elements |
| `layout.quality` | 0..1 health score + issue list |
| `layout.fidelity` | Which CSS features are exact vs approximated |

### 3. HTTP API — call from anything

```bash
cornea --serve-http [addr]        # default 127.0.0.1:8080
```

```bash
curl -s -X POST http://127.0.0.1:8080/inspect \
  -H 'Content-Type: application/json' \
  -d '{"html":"<p style=\"color:#cccccc\">bady</p>","width":360}'
```

```text
{"total_elements":1,"contrast":[{"selector":"...>p","fg":"#cccccc","bg":"#ffffff",
                                 "ratio":1.61,"pass_aa":false}]}
```

| Route | Method | Returns |
|-------|--------|---------|
| `/inspect` | POST | Full report |
| `/overlaps` | POST | Collisions |
| `/overflow` | POST | Clipped / collapsed / off-screen |
| `/contrast` | POST | WCAG ratios |
| `/quality` | POST | Health score |
| `/fidelity` | GET | Engine capabilities |
| `/health` | GET | Liveness |

---

## Architecture

```
                    ┌─────────────────────────────────────┐
   HTML + CSS ──►   │           cornea (one binary)       │
                    │                                     │
                    │   dom.rs      html5ever ──► tree    │
                    │   css.rs      <style> + inline      │
                    │                                    │
                    │   layout.rs   deterministic layout  │
                    │               (block/inline/flex,   │
                    │                box model, z-index)  │
                    │                    │                │
                    │   model.rs    Visual Geometry Model │
                    │                    │                │
                    │   inspect.rs  overlap / overflow /  │
                    │               contrast / quality    │
                    │                    │                │
                    │   rest.rs     canonical dispatch    │
                    └───────┬─────────────┬───────────────┘
                            │             │
                      ┌─────┴────┐   ┌────┴───────────────┐
                      │  CLI     │   │  MCP (stdio) / HTTP│
                      │  cornea  │   │  layout.* / REST   │
                      └──────────┘   └────────────────────┘
```

**Read the full technical spec:** [`CORNEA-ARCHITECTURE.md`](./CORNEA-ARCHITECTURE.md)

---

## Fidelity — honest about what's approximate

Cornea never silently fakes precision. `layout.fidelity` tells an agent exactly what it can trust:

```json
{
  "exact":        ["box model", "block flow", "inline text estimates", "flex row/column (no wrap)",
                   "z-index", "visibility", "absolute/fixed left/top", "inline styles",
                   "class/id/tag selectors", "WCAG contrast (hex + named colors)"],
  "approximate":  ["text glyph width (not shaping)", "flex-grow/flex-basis distribution",
                   "grid", "media queries", "border-radius", "percentage widths"],
  "deferred":     ["JS/DOM scripting", "external stylesheets (<link>)", "complex selectors (combinators, pseudo)"]
}
```

---

## Visual guide: testing (battle-tested)

27 tests run clean with `cargo test`; CI enforces **fmt, clippy `-D warnings`, release build, tests, and a CLI smoke test** on every push.

```text
$ cargo test
Running unittests src/lib.rs      ... 18 passed   // determinism, overlap, overflow,
Running unittests src/main.rs     ...  4 passed    //   contrast, inline flow, flex,
Running tests/endpoints.rs        ...  5 passed    //   nesting, empty input, MCP, CLI
```

- **Determinism** — same page inspected twice gives byte-identical JSON (the core thesis).
- **Bug detection** — the fixture's overlaps, overflows, and contrast failures are all asserted.
- **End-to-end** — the compiled binary is spawned and `layout.*` is called over real stdio MCP.
- **HTTP** — the TCP server boots on an ephemeral port and real requests are made.
- **Edge cases** — empty HTML, deep nesting (no crash), `display:none`, long text, flex row/col, inline wrapping.

---

## Layout support (v1 scope)

| Feature | Status |
|---------|--------|
| Block flow, box model (content/border-box) | ✅ exact |
| Inline text runs (horizontal, wrapping) | ✅ |
| Flex row / column (simplified, no wrap) | ✅ approximate |
| Absolute / fixed positioning (left/top) | ✅ |
| z-index, visibility, `display:none` | ✅ |
| `.class` / `#id` / `tag` selectors + inline styles | ✅ |
| WCAG AA contrast (hex + 18 named colors) | ✅ |
| Grid, media queries, external `<link>` CSS | ⏳ deferred |

---

## Repository layout

```
cornea/
├── Cargo.toml             # crate: html5ever + serde/serde_json, release LTO+strip
├── README.md              # this file
├── CORNEA-ARCHITECTURE.md # full technical spec
├── src/
│   ├── lib.rs             # build_model / analyze pipeline + unit tests
│   ├── dom.rs             # html5ever -> lightweight element tree
│   ├── css.rs             # <style> + inline style resolution
│   ├── layout.rs          # deterministic layout engine
│   ├── model.rs           # VisualModel / ElementView / Rect
│   ├── inspect.rs         # overlap / overflow / contrast / quality
│   ├── rest.rs            # canonical endpoint dispatch (shared by all surfaces)
│   ├── main.rs            # CLI + stdio MCP server
│   └── server_http.rs     # dependency-free HTTP/1.1 API
├── tests/
│   ├── endpoints.rs       # end-to-end CLI + MCP binary tests
│   └── fixtures/sample-bugs.html  # known-bug fixture for CI smoke
└── .github/workflows/ci.yml
```

---

## Status

**Working MVP, hardened and battle-tested.** Deterministic inspection engine with CLI + MCP + HTTP API, 27 passing tests, green CI. The roadmap builds toward giving Cornea (and the sibling **Crayon** text-to-image project) an even richer perception over subsequent phases.

## Publishing

Cornea is distributed three ways: **crates.io** (`cargo install cornea`), **GitHub Release + Homebrew** (`brew install`), and the **MCP Registry** (`layout.*` tools discoverable by agents). All are staged in this repo — see **[`PUBLISHING.md`](./PUBLISHING.md)** for the tokens, `server.json` manifest, and the release/tag recipe.

## License

MIT. See [`LICENSE`](./LICENSE).
