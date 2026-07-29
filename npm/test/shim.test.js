"use strict";

const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { selectPlatform, resolvePackageDir } = require("../bin/adfc.js");
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

test("resolvePackageDir finds a sibling package layout", () => {
  // Mirrors node_modules/adfc/bin/adfc.js next to node_modules/adfc-linux-x64.
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "adfc-resolve-"));
  const sibling = path.join(root, "adfc-linux-x64");
  fs.mkdirSync(sibling, { recursive: true });
  fs.writeFileSync(path.join(sibling, "package.json"), "{}");

  const shimDir = path.join(root, "adfc", "bin");
  fs.mkdirSync(shimDir, { recursive: true });

  // resolvePackageDir walks up two levels from the shim's own directory, so
  // exercise it from a copy placed in that layout.
  fs.copyFileSync(
    path.join(__dirname, "..", "bin", "adfc.js"),
    path.join(shimDir, "adfc.js"),
  );
  const relocated = require(path.join(shimDir, "adfc.js"));
  assert.equal(relocated.resolvePackageDir("adfc-linux-x64"), sibling);

  fs.rmSync(root, { recursive: true, force: true });
});

test("resolvePackageDir reports a missing package clearly", () => {
  assert.throws(
    () => resolvePackageDir("adfc-definitely-not-installed"),
    /is not installed/,
  );
});
