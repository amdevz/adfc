#!/usr/bin/env node
"use strict";

// Entry point for the `adfc` npm package.
//
// The binary lives in a per-platform package listed in optionalDependencies,
// so npm installs exactly one; this finds it and hands over. Binaries ship
// inside those tarballs rather than being fetched during install, which is
// what keeps `npm ci --ignore-scripts`, offline caches and mirrored registries
// working.

const { existsSync, chmodSync } = require("fs");
const { dirname, join, resolve } = require("path");
const { spawnSync } = require("child_process");
const { createRequire } = require("module");

/**
 * Choose the platform package for a given process.platform / process.arch.
 *
 * Exported so the table can be tested without pretending to be another
 * operating system.
 */
function selectPlatform(platforms, platform, arch) {
  const match = platforms.find(
    (spec) => spec.os.includes(platform) && spec.cpu.includes(arch),
  );
  if (match) return match;

  throw new Error(
    `unsupported platform: ${platform} ${arch}. Prebuilt binaries cover ` +
      "linux, darwin and win32 on x64 and arm64; build from source with " +
      "`cargo install adfc` instead.",
  );
}

/**
 * Find the executable inside its platform package.
 *
 * Two candidate locations, because resolution alone is not enough: under some
 * pnpm and Yarn PnP layouts the package is installed but not resolvable from
 * this file, and the sibling directory covers those. Each candidate is checked
 * for the binary itself rather than the manifest, so a half-installed package
 * reads as missing here instead of failing later at exec.
 */
function locateBinary(spec) {
  const candidates = [];

  try {
    const manifest = createRequire(__filename).resolve(
      `${spec.packageName}/package.json`,
    );
    candidates.push(dirname(manifest));
  } catch {
    // Not resolvable from here; the sibling layout below may still find it.
  }
  candidates.push(resolve(__dirname, "..", "..", spec.packageName));

  for (const dir of candidates) {
    const binary = join(dir, spec.binaryName);
    if (existsSync(binary)) return binary;
  }

  throw new Error(
    `platform package ${spec.packageName} is not installed, or is missing ` +
      `${spec.binaryName}. Reinstall adfc for this platform. Looked in: ` +
      candidates.join(", "),
  );
}

function main() {
  const platforms = require("../platforms.json");
  const spec = selectPlatform(platforms, process.platform, process.arch);
  const binary = locateBinary(spec);

  // npm does not reliably preserve the executable bit through a tarball.
  // Setting it unconditionally avoids the gap between testing and fixing it,
  // and a read-only install is already executable, so a failure here is not
  // worth reporting.
  if (process.platform !== "win32") {
    try {
      chmodSync(binary, 0o755);
    } catch {
      // Already executable, or not ours to change. Let exec decide.
    }
  }

  const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
  if (result.error) throw result.error;

  // Re-raise a signal rather than turning it into an exit code: a caller that
  // killed this process expects to observe the signal, and the binary's own
  // broken-pipe handling depends on reaching the shell intact.
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

module.exports = { selectPlatform, locateBinary };
