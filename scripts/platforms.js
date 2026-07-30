"use strict";

// The one place a Rust target is mapped to an npm platform package.
//
// The build script generates package manifests from this, and serialises it
// into the entry package as platforms.json for the runtime shim to read. Both
// sides therefore cannot disagree about which package holds which binary.
//
// Platform packages live under a scope so a future target's name is reserved
// by construction and one scope-level npm token covers packages that do not
// exist yet. The entry package stays unscoped: `adfc` is what users type, and
// it matches the crate and the binary.
//
// Linux is served a single statically linked musl build with no `libc` field,
// so npm installs it on glibc distributions too. A static binary runs fine
// there, which means the alternative — shipping glibc and musl variants and
// detecting which is needed at runtime — would double the packages and add a
// detection path that can only ever be wrong.

const PLATFORMS = [
  {
    rustTarget: "x86_64-unknown-linux-musl",
    packageName: "@amdevz/adfc-linux-x64",
    archive: "adfc-x86_64-unknown-linux-musl.tar.gz",
    binaryName: "adfc",
    os: ["linux"],
    cpu: ["x64"],
  },
  {
    rustTarget: "aarch64-unknown-linux-musl",
    packageName: "@amdevz/adfc-linux-arm64",
    archive: "adfc-aarch64-unknown-linux-musl.tar.gz",
    binaryName: "adfc",
    os: ["linux"],
    cpu: ["arm64"],
  },
  {
    rustTarget: "x86_64-apple-darwin",
    packageName: "@amdevz/adfc-darwin-x64",
    archive: "adfc-x86_64-apple-darwin.tar.gz",
    binaryName: "adfc",
    os: ["darwin"],
    cpu: ["x64"],
  },
  {
    rustTarget: "aarch64-apple-darwin",
    packageName: "@amdevz/adfc-darwin-arm64",
    archive: "adfc-aarch64-apple-darwin.tar.gz",
    binaryName: "adfc",
    os: ["darwin"],
    cpu: ["arm64"],
  },
  {
    rustTarget: "x86_64-pc-windows-msvc",
    packageName: "@amdevz/adfc-win32-x64",
    archive: "adfc-x86_64-pc-windows-msvc.zip",
    binaryName: "adfc.exe",
    os: ["win32"],
    cpu: ["x64"],
  },
  {
    rustTarget: "aarch64-pc-windows-msvc",
    packageName: "@amdevz/adfc-win32-arm64",
    archive: "adfc-aarch64-pc-windows-msvc.zip",
    binaryName: "adfc.exe",
    os: ["win32"],
    cpu: ["arm64"],
  },
];

/// The subset the runtime shim needs; the rest is build-time only.
function runtimeConfig(platforms = PLATFORMS) {
  return platforms.map(({ packageName, binaryName, os, cpu }) => ({
    packageName,
    binaryName,
    os,
    cpu,
  }));
}

module.exports = { PLATFORMS, runtimeConfig };
