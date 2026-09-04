"use strict";
// Unit tests for the installer mapping. No dependencies. Run: npm test
const assert = require("assert");
const { assetFor } = require("./install.js");

assert.strictEqual(assetFor("linux-x64"), "cornea-x86_64-unknown-linux-gnu");
assert.strictEqual(assetFor("linux-arm64"), "cornea-aarch64-unknown-linux-gnu");
assert.strictEqual(assetFor("darwin-arm64"), "cornea-aarch64-apple-darwin");
assert.strictEqual(assetFor("darwin-x64"), "cornea-x86_64-apple-darwin");
assert.strictEqual(assetFor("win32-x64"), "cornea-x86_64-pc-windows-msvc.exe");
assert.strictEqual(assetFor("android-arm64"), null);
assert.strictEqual(assetFor("freebsd-x64"), null);

console.log("cornea installer mapping: 7/7 ok");
