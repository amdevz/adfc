"use strict";

const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { selectPlatform, locateBinary } = require("../bin/adfc.js");
const { runtimeConfig } = require("../../scripts/platforms.js");

const PLATFORMS = runtimeConfig();

test("selects the right package for every supported platform", () => {
  const cases = [
    ["linux", "x64", "adfc-linux-x64"],
    ["linux", "arm64", "adfc-linux-arm64"],
    ["darwin", "x64", "adfc-darwin-x64"],
    ["darwin", "arm64", "adfc-darwin-arm64"],
    ["win32", "x64", "adfc-win32-x64"],
    ["win32", "arm64", "adfc-win32-arm64"],
  ];
  for (const [platform, arch, expected] of cases) {
    assert.equal(
      selectPlatform(PLATFORMS, platform, arch).packageName,
      expected,
      `${platform} ${arch}`,
    );
  }
});

test("linux resolves to the musl build regardless of libc", () => {
  // There is deliberately no libc dimension: the Linux binary is static, so
  // one package serves glibc and musl hosts alike.
  const spec = selectPlatform(PLATFORMS, "linux", "x64");
  assert.equal(spec.packageName, "adfc-linux-x64");
  assert.ok(!("libc" in spec), "runtime config should carry no libc field");
});

test("windows packages carry the .exe binary name", () => {
  assert.equal(selectPlatform(PLATFORMS, "win32", "x64").binaryName, "adfc.exe");
  assert.equal(selectPlatform(PLATFORMS, "linux", "x64").binaryName, "adfc");
});

test("unsupported platform names it and points at cargo", () => {
  assert.throws(
    () => selectPlatform(PLATFORMS, "freebsd", "riscv64"),
    (error) => {
      assert.match(error.message, /freebsd/);
      assert.match(error.message, /riscv64/);
      assert.match(error.message, /cargo install adfc/);
      return true;
    },
  );
});

test("unsupported arch on a supported os is still rejected", () => {
  assert.throws(() => selectPlatform(PLATFORMS, "linux", "ia32"), /unsupported platform/);
});

test("locateBinary finds a sibling package layout", () => {
  // Mirrors node_modules/adfc/bin/adfc.js next to node_modules/adfc-linux-x64.
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "adfc-locate-"));
  const sibling = path.join(root, "adfc-linux-x64");
  fs.mkdirSync(sibling, { recursive: true });
  fs.writeFileSync(path.join(sibling, "adfc"), "#!/bin/sh\ntrue\n");

  const shimDir = path.join(root, "adfc", "bin");
  fs.mkdirSync(shimDir, { recursive: true });
  fs.copyFileSync(
    path.join(__dirname, "..", "bin", "adfc.js"),
    path.join(shimDir, "adfc.js"),
  );

  const relocated = require(path.join(shimDir, "adfc.js"));
  const spec = selectPlatform(PLATFORMS, "linux", "x64");
  assert.equal(relocated.locateBinary(spec), path.join(sibling, "adfc"));

  fs.rmSync(root, { recursive: true, force: true });
});

test("a package present but missing its binary reads as not installed", () => {
  // Checking for the binary rather than the manifest is what turns a
  // half-installed package into a clear error here instead of a failure at exec.
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "adfc-partial-"));
  const sibling = path.join(root, "adfc-linux-x64");
  fs.mkdirSync(sibling, { recursive: true });
  fs.writeFileSync(path.join(sibling, "package.json"), "{}");

  const shimDir = path.join(root, "adfc", "bin");
  fs.mkdirSync(shimDir, { recursive: true });
  fs.copyFileSync(
    path.join(__dirname, "..", "bin", "adfc.js"),
    path.join(shimDir, "adfc.js"),
  );

  const relocated = require(path.join(shimDir, "adfc.js"));
  assert.throws(
    () => relocated.locateBinary(selectPlatform(PLATFORMS, "linux", "x64")),
    /is missing adfc/,
  );

  fs.rmSync(root, { recursive: true, force: true });
});

test("locateBinary reports a missing package clearly", () => {
  assert.throws(
    () => locateBinary({ packageName: "adfc-not-installed", binaryName: "adfc" }),
    /is not installed/,
  );
});
