#!/usr/bin/env node
"use strict";

// Turn dist's release archives into the seven npm package trees: one entry
// package plus one per platform.
//
// Usage:
//   node scripts/build-npm-packages.js --plan <plan.json> \
//        [--artifacts-dir target/distrib] [--out-dir npm/.output] [--only <target>]
//
// This packages archives; it does not build them. Run `dist build` first, or
// download the release artifacts.

const fs = require("fs");
const os = require("os");
const path = require("path");
const { execFileSync } = require("child_process");
const { PLATFORMS, runtimeConfig } = require("./platforms.js");

const REPO_ROOT = path.resolve(__dirname, "..");

// Everything ships under one scope, so a single scope-level npm token covers
// the whole release. The command this installs is still `adfc`: the bin name in
// a package manifest is independent of the package name, so `adfc` stays what a
// user types and what the crate and binary are called.
const ENTRY_PACKAGE = "@amdevz/adfc";

function parseArgs(argv) {
  const args = {
    plan: null,
    artifactsDir: path.join(REPO_ROOT, "target", "distrib"),
    outDir: path.join(REPO_ROOT, "npm", ".output"),
    only: null,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    const value = argv[i + 1];
    if (flag === "--plan") (args.plan = value), (i += 1);
    else if (flag === "--artifacts-dir") (args.artifactsDir = value), (i += 1);
    else if (flag === "--out-dir") (args.outDir = value), (i += 1);
    // Packaging only the host target is what makes a local end-to-end check
    // possible without cross-compiling all six.
    else if (flag === "--only") (args.only = value), (i += 1);
    else throw new Error(`unknown argument: ${flag}`);
  }
  if (!args.plan) throw new Error("--plan <dist-plan.json> is required");
  return args;
}

/**
 * The version, taken from dist's plan.
 *
 * Never accepted as an argument: the plan derives it from Cargo.toml, and a
 * second source could disagree with the crate being packaged.
 */
function readVersion(planPath) {
  const plan = JSON.parse(fs.readFileSync(planPath, "utf8"));
  const versions = new Set(
    plan.releases.filter((r) => r.app_name === "adfc").map((r) => r.app_version),
  );
  if (versions.size !== 1) {
    throw new Error(
      `expected exactly one adfc version in the plan, found: ${[...versions].join(", ") || "none"}`,
    );
  }
  return [...versions][0];
}

/** Extract one named binary from a .tar.gz or .zip release archive. */
function extractBinary(archivePath, binaryName) {
  if (!fs.existsSync(archivePath)) {
    throw new Error(
      `missing release archive ${archivePath}. Run \`dist build\` first, or pass --artifacts-dir.`,
    );
  }
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "adfc-npm-"));
  try {
    if (archivePath.endsWith(".zip")) {
      execFileSync("unzip", ["-q", "-o", archivePath, "-d", scratch]);
    } else {
      execFileSync("tar", ["-xzf", archivePath, "-C", scratch]);
    }
    const found = findFile(scratch, binaryName);
    if (!found) {
      throw new Error(`${binaryName} not found inside ${path.basename(archivePath)}`);
    }
    return fs.readFileSync(found);
  } finally {
    fs.rmSync(scratch, { recursive: true, force: true });
  }
}

/** dist nests the binary under a directory named after the archive. */
function findFile(dir, name) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      const nested = findFile(full, name);
      if (nested) return nested;
    } else if (entry.name === name) {
      return full;
    }
  }
  return null;
}

function commonManifestFields() {
  return {
    license: "MIT",
    repository: { type: "git", url: "git+https://github.com/amdevz/adfc.git" },
    homepage: "https://github.com/amdevz/adfc",
    bugs: { url: "https://github.com/amdevz/adfc/issues" },
    engines: { node: ">=18" },
    // Yarn PnP keeps packages zipped by default, and a zipped native binary
    // cannot be executed.
    preferUnplugged: true,
  };
}

function writePlatformPackage(spec, version, artifactsDir, outDir) {
  const dir = path.join(outDir, spec.packageName);
  fs.mkdirSync(dir, { recursive: true });

  const binary = extractBinary(path.join(artifactsDir, spec.archive), spec.binaryName);
  const binaryPath = path.join(dir, spec.binaryName);
  fs.writeFileSync(binaryPath, binary);
  if (!spec.binaryName.endsWith(".exe")) fs.chmodSync(binaryPath, 0o755);

  fs.copyFileSync(path.join(REPO_ROOT, "LICENSE"), path.join(dir, "LICENSE"));
  fs.writeFileSync(
    path.join(dir, "README.md"),
    `# ${spec.packageName}\n\nPlatform binary for [adfc](https://github.com/amdevz/adfc).\n` +
      `Not meant to be installed directly; install \`adfc\` instead.\n`,
  );

  writeJson(path.join(dir, "package.json"), {
    name: spec.packageName,
    version,
    description: `Native ${spec.os.join("/")} ${spec.cpu.join("/")} binary for adfc.`,
    ...commonManifestFields(),
    os: spec.os,
    cpu: spec.cpu,
    files: [spec.binaryName, "README.md", "LICENSE"],
  });

  return dir;
}

function writeEntryPackage(platforms, version, outDir) {
  const dir = path.join(outDir, ENTRY_PACKAGE);
  fs.mkdirSync(path.join(dir, "bin"), { recursive: true });

  fs.copyFileSync(
    path.join(REPO_ROOT, "npm", "bin", "adfc.js"),
    path.join(dir, "bin", "adfc.js"),
  );
  fs.chmodSync(path.join(dir, "bin", "adfc.js"), 0o755);
  writeJson(path.join(dir, "platforms.json"), runtimeConfig(platforms));

  fs.copyFileSync(path.join(REPO_ROOT, "LICENSE"), path.join(dir, "LICENSE"));
  fs.copyFileSync(path.join(REPO_ROOT, "README.md"), path.join(dir, "README.md"));

  // Exact versions, never ranges: a range lets npm pair the entry package with
  // a platform package built from different sources.
  const optionalDependencies = Object.fromEntries(
    platforms.map((spec) => [spec.packageName, version]),
  );

  writeJson(path.join(dir, "package.json"), {
    name: ENTRY_PACKAGE,
    version,
    description:
      "Convert Markdown to Atlassian Document Format (ADF), with JSON Schema validation",
    keywords: ["adf", "atlassian", "markdown", "confluence", "jira"],
    ...commonManifestFields(),
    bin: { adfc: "bin/adfc.js" },
    files: ["bin", "platforms.json", "README.md", "LICENSE"],
    optionalDependencies,
  });

  return dir;
}

/**
 * Every package directory under `outDir`.
 *
 * Scoped names nest a level (`@amdevz/adfc-linux-x64`), so a flat readdir
 * would miss them. Exported so the tests, the release workflow and the
 * end-to-end script all agree on the layout instead of each globbing for it.
 */
function packageDirs(outDir) {
  const dirs = [];
  for (const entry of fs.readdirSync(outDir, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const full = path.join(outDir, entry.name);
    if (entry.name.startsWith("@")) {
      for (const scoped of fs.readdirSync(full, { withFileTypes: true })) {
        if (scoped.isDirectory()) dirs.push(path.join(full, scoped.name));
      }
    } else {
      dirs.push(full);
    }
  }
  return dirs.sort();
}

function writeJson(file, value) {
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`);
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const version = readVersion(args.plan);

  const platforms = args.only
    ? PLATFORMS.filter((p) => p.rustTarget === args.only)
    : PLATFORMS;
  if (platforms.length === 0) {
    throw new Error(`--only ${args.only} matched no known target`);
  }

  fs.rmSync(args.outDir, { recursive: true, force: true });
  fs.mkdirSync(args.outDir, { recursive: true });

  for (const spec of platforms) {
    writePlatformPackage(spec, version, args.artifactsDir, args.outDir);
    console.log(`  ${spec.packageName}@${version}`);
  }
  // The entry package always lists all six, even on a partial build: its
  // optionalDependencies describe the release, not what happened to be packaged.
  writeEntryPackage(PLATFORMS, version, args.outDir);
  console.log(`  ${ENTRY_PACKAGE}@${version} (entry)`);
  console.log(`\nwrote ${platforms.length + 1} package(s) to ${args.outDir}`);
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    console.error(`build-npm-packages: ${error.message}`);
    process.exit(1);
  }
}

module.exports = {
  ENTRY_PACKAGE,
  readVersion,
  extractBinary,
  writePlatformPackage,
  writeEntryPackage,
  packageDirs,
};
