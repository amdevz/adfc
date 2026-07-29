#!/usr/bin/env node
"use strict";

// Entry point for the `adfc` npm package.
//
// The binary itself lives in a per-platform package listed in
// optionalDependencies, so npm installs exactly one and this resolves it and
// execs it. The binary ships inside that package's tarball rather than being
// downloaded on install, which is what lets `npm ci --ignore-scripts`, offline
// caches and mirrored registries work.

const fs = require("fs");
const path = require("path");
const { spawnSync } = require("child_process");
const { createRequire } = require("module");

/**
 * Pick the platform package for a given process.platform / process.arch.
 *
 * Exported so the selection table can be tested without spawning anything or
 * pretending to be another operating system.
 */
function selectPlatform(platforms, platform, arch) {
  const match = platforms.find(
    (spec) => spec.os.includes(platform) && spec.cpu.includes(arch),
  );
  if (!match) {
    throw new Error(
      `unsupported platform: ${platform} ${arch}. ` +
        "Prebuilt binaries cover linux, darwin and win32 on x64 and arm64; " +
        "build from source with `cargo install adfc` instead.",
    );
  }
  return match;
}

/**
 * Locate an installed platform package.
 *
 * Two strategies because one is not enough in practice: `require.resolve`
 * fails under some pnpm and Yarn PnP layouts where the package is present but
 * not resolvable from here, and the sibling-directory guess covers those.
 */
function resolvePackageDir(packageName) {
  const attempts = [];

  try {
    const resolved = createRequire(__filename).resolve(
      `${packageName}/package.json`,
    );
    if (fs.existsSync(resolved)) return path.dirname(resolved);
    attempts.push(resolved);
  } catch (error) {
    attempts.push(String(error.message ?? error));
  }

  const sibling = path.resolve(__dirname, "..", "..", packageName);
  if (fs.existsSync(path.join(sibling, "package.json"))) return sibling;
  attempts.push(sibling);

  throw new Error(
    `platform package ${packageName} is not installed. ` +
      `Reinstall adfc for this platform. Tried: ${attempts.join(", ")}`,
  );
}

/** npm does not always preserve the executable bit through a tarball. */
function ensureExecutable(binaryPath) {
  if (process.platform === "win32") return;
  try {
    fs.accessSync(binaryPath, fs.constants.X_OK);
  } catch {
    fs.chmodSync(binaryPath, 0o755);
  }
}

function main() {
  const platforms = require("../platforms.json");
  const spec = selectPlatform(platforms, process.platform, process.arch);
  const binary = path.join(resolvePackageDir(spec.packageName), spec.binaryName);

  ensureExecutable(binary);

  const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
  if (result.error) throw result.error;

  // Re-raise rather than translating to an exit code: a caller that killed
  // this process expects to see the signal, and `adfc | head` relies on the
  // binary's own broken-pipe handling reaching the shell intact.
  if (result.signal) {
    process.kill(process.pid, result.signal);
    return;
  }
  process.exit(result.status ?? 1);
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    console.error(`adfc: ${error.message}`);
    process.exit(1);
  }
}

module.exports = { selectPlatform, resolvePackageDir };
