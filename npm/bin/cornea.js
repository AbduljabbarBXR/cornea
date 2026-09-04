#!/usr/bin/env node
/* cornea launcher. Runs the platform binary downloaded by postinstall. */
"use strict";

const { spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");

function binaryPath() {
  const exe = process.platform === "win32" ? "cornea.exe" : "cornea";
  return path.join(__dirname, exe);
}

function main() {
  const bin = binaryPath();
  if (!fs.existsSync(bin)) {
    console.error("cornea: binary not found. Reinstall with: npm install -g optrex");
    process.exit(1);
  }
  const result = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
  if (result.error) {
    console.error(`cornea: could not run the binary (${result.error.message}).`);
    console.error("Your platform may need a different build:");
    console.error("https://github.com/AbduljabbarBXR/cornea/releases");
    process.exit(1);
  }
  process.exit(result.status === null ? 1 : result.status);
}

if (require.main === module) main();

module.exports = { binaryPath };
