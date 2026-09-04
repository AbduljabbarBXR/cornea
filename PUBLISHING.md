# Publishing Cornea

Cornea is distributed through **four independent channels**, each installed from the terminal like any other tool. You can ship them together or one at a time.

| Channel | Result | One-line install |
|---------|--------|------------------|
| npm | `cornea` command wrapper | `npm install -g optrex` |
| crates.io | Rust crate + binary | `cargo install cornea` / `cargo binstall cornea` |
| GitHub Release + Homebrew | Fast native binaries | `brew install AbduljabbarBXR/tap/cornea` |
| MCP Registry | Agents discover `layout.*` | `npx @smithery/cli install ...` |

All four are driven by **tagging a version**. Everything is staged in this repo; publishing just needs tokens + a `git tag`.

---

## 0. Before you start

- [ ] Confirm the **name `cornea` is available** on crates.io: `curl -s https://crates.io/api/v1/crates/cornea` returns `404`.
- [ ] Decide the **license**. `Cargo.toml` sets `MIT` (crates.io requires a license to publish). Update `README.md`'s "All rights reserved" line and add a `LICENSE` if you open it up.
- [ ] Make the repo **public** (optional, but recommended). `cargo publish`, Homebrew taps, and most MCP directories surface a public source.

---

## 1. Crates.io (`cargo install cornea`)

Metadata is already in `Cargo.toml` (license, description, repository, keywords, categories, readme). Verify the package:

```bash
cargo package --allow-dirty        # builds target/package/cornea-<ver>.crate
```

Authenticate once, then publish a new version on every release:

```bash
cargo login                        # paste your crates.io API token
cargo publish
```

Then anyone installs with:

```bash
cargo install cornea               # compiles from source (universal)
cargo binstall cornea              # fast, downloads the prebuilt GitHub binary (needs release workflow built first)
```

> `cargo install` works for everyone on any platform. `binstall` is instant but needs the GitHub Release from step 2.

---

## 2. GitHub Release + Homebrew (`brew install`)

A release workflow (`.github/workflows/release.yml`) is committed. On a `v*` tag it:

1. Builds release binaries for **Linux x86_64/arm64, macOS x86_64/arm64, Windows x86_64**.
2. Uploads them to a **GitHub Release** (the `taiki-e/upload-rust-binary` action).
3. Publishes the crate to **crates.io** (needs `CARGO_REGISTRY_TOKEN` repo secret).

### Trigger a release

```bash
git tag v0.1.0
git push origin v0.1.0
```

### Homebrew tap

Homebrew taps live in **their own repo**. Two options:

**A. Manual local formula (works now):**
```bash
brew install --formula contrib/brew/cornea.rb
```

**B. A publishable tap**. Create a second repo `AbduljabbarBXR/homebrew-cornea` containing `Formula/cornea.rb` (copy `contrib/brew/cornea.rb`), then update the two `url`/`sha256` placeholders per version. Then:

```bash
brew tap AbduljabbarBXR/cornea
brew install cornea
```

For **automatic tap updates**, use [`cargo-dist`](https://opensource.axo.dev/cargo-dist/). Run `cargo dist init` once and it maintains the tap repo + formula for you on every tag.

---

## 3. MCP Registry (`server.json`)

`server.json` (committed, in the official MCP Registry schema) declares Cornea as a **stdio** server running `cornea --serve`, with all six `layout.*` tools.

The MCP ecosystem consolidated (2026) around the **official registry** at `registry.modelcontextprotocol.io`. Publish there first, and it propagates to Smithery, PulseMCP, Glama, etc.

```bash
curl -L -sSf https://github.com/modelcontextprotocol/registry/releases/latest/download/mcp-publisher-linux-x86_64.tar.gz | tar xz  # install mcp-publisher

mcp-publisher init                 # scaffold (ours is already at server.json)
mcp-publisher login github         # prove namespace io.github.AbduljabbarBXR/
mcp-publisher publish server.json
```

> GitHub auth requires the server name to start with `io.github.ABDULJABBARBXR/`. The committed `server.json` already uses that namespace format. Keep the username casing consistent with your GitHub login.

### Smithery (optional one-click install)

Smithery auto-indexes GitHub repos with an MCP manifest on tag, and distributes a local **`.mcpb` bundle** for stdio servers:

```bash
npx -y @smithery/cli install io.github.AbduljabbarBXR/cornea --client claude
npx -y @smithery/cli install io.github.AbduljabbarBXR/cornea --client claude-code
npx -y @smithery/cli install io.github.AbduljabbarBXR/cornea --client cursor
```

---

## Secret checklist

| Secret | Used by |
|--------|---------|
| `CARGO_REGISTRY_TOKEN` | release.yml `publish-crates` step |
| (optional) `GITHUB_TOKEN` | auto-provided; uploads binaries |

## Quick release recipe

```bash
# bump version
sed -i 's/^version = .*/version = "0.1.1"/' Cargo.toml server.json
cargo package --allow-dirty          # sanity check
git add -A && git commit -m "chore: release 0.1.1"
git tag v0.1.1 && git push origin main --tags
```

Then `cargo publish` runs in CI; `brew`/`cargo binstall` consume the resulting GitHub Release.
