# optrex (Cornea npm installer)

Installs the prebuilt [Cornea](https://github.com/AbduljabbarBXR/cornea) binary for your platform and exposes the `cornea` command. Cornea is deterministic visual inspection for AI agents: overlap, overflow, contrast and quality as token-cheap geometry instead of pixel dumps. No Chromium.

```bash
npm install -g optrex
cornea --help
```

On install, the matching binary is downloaded from GitHub Releases into the package `bin/` folder. No Rust toolchain needed.

## Supported platforms

| OS | Arch | Asset |
|----|------|-------|
| Linux (glibc) | x64 | `cornea-x86_64-unknown-linux-gnu` |
| Linux (glibc) | arm64 | `cornea-aarch64-unknown-linux-gnu` |
| macOS | arm64 | `cornea-aarch64-apple-darwin` |
| macOS | x64 | `cornea-x86_64-apple-darwin` |
| Windows | x64 | `cornea-x86_64-pc-windows-msvc.exe` |

Not listed, e.g. Android, or musl/Alpine: the installer stops with a clear error. Install from source instead (`cargo install cornea`) or pick a build from the [releases page](https://github.com/AbduljabbarBXR/cornea/releases).

## Usage

Same as the native binary:

```bash
cornea page.html 360
cornea --serve   # MCP server over stdio for agents
```

## Versions

npm package version tracks the Cornea release version. `optrex@0.2.2` installs Cornea 0.2.2.

## Uninstall

```bash
npm uninstall -g optrex
```

## License

MIT. See [LICENSE](./LICENSE). Binary builds follow the [Cornea repo license](https://github.com/AbduljabbarBXR/cornea).
