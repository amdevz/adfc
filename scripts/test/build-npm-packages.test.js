"use strict";

const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const REPO_ROOT = path.resolve(__dirname, "..", "..");
const SCRIPT = path.join(REPO_ROOT, "scripts", "build-npm-packages.js");
const { PLATFORMS } = require("../platforms.js");
const { packageDirs, ENTRY_PACKAGE } = require("../build-npm-packages.js");

/** A scratch directory holding a fake plan and fake release archives. */
function makeFixtures(version = "9.9.9") {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "adfc-pkg-test-"));
  const artifacts = path.join(root, "artifacts");
  fs.mkdirSync(artifacts, { recursive: true });

  const planPath = path.join(root, "plan.json");
  fs.writeFileSync(
    planPath,
    JSON.stringify({
      releases: [{ app_name: "adfc", app_version: version }],
    }),
  );

  // dist nests the binary under a directory named after the archive, so the
  // fixtures reproduce that shape rather than a flat archive.
  for (const spec of PLATFORMS) {
    const stem = spec.archive.replace(/\.tar\.gz$|\.zip$/, "");
    const staging = path.join(root, "staging", stem);
    fs.mkdirSync(staging, { recursive: true });
    fs.writeFileSync(path.join(staging, spec.binaryName), `fake ${spec.rustTarget}`);
    fs.writeFileSync(path.join(staging, "README.md"), "fixture");

    const out = path.join(artifacts, spec.archive);
    if (spec.archive.endsWith(".zip")) {
      execFileSync("zip", ["-qr", out, stem], { cwd: path.join(root, "staging") });
    } else {
      execFileSync("tar", ["-czf", out, stem], { cwd: path.join(root, "staging") });
    }
  }

  return { root, artifacts, planPath, version };
}

function run(fixtures, extraArgs = []) {
  const outDir = path.join(fixtures.root, "out");
  execFileSync(
    process.execPath,
    [
      SCRIPT,
      "--plan",
      fixtures.planPath,
      "--artifacts-dir",
      fixtures.artifacts,
      "--out-dir",
      outDir,
      ...extraArgs,
    ],
    { stdio: "pipe" },
  );
  return outDir;
}

const manifest = (out, pkg) =>
  JSON.parse(fs.readFileSync(path.join(out, pkg, "package.json"), "utf8"));

/** Package name -> its emitted directory, regardless of scoping. */
const byName = (out) =>
  Object.fromEntries(
    packageDirs(out).map((dir) => [
      JSON.parse(fs.readFileSync(path.join(dir, "package.json"), "utf8")).name,
      dir,
    ]),
  );

test("emits seven packages: one entry plus six platforms", () => {
  const f = makeFixtures();
  const out = run(f);
  const found = byName(out);
  assert.equal(Object.keys(found).length, 7, `got ${Object.keys(found).join(", ")}`);
  assert.ok(ENTRY_PACKAGE in found, ENTRY_PACKAGE);
  for (const spec of PLATFORMS) assert.ok(spec.packageName in found, spec.packageName);
  fs.rmSync(f.root, { recursive: true, force: true });
});

test("entry package pins all six platforms as exact optional dependencies", () => {
  const f = makeFixtures();
  const out = run(f);
  const entry = manifest(out, ENTRY_PACKAGE);

  assert.equal(Object.keys(entry.optionalDependencies).length, 6);
  for (const spec of PLATFORMS) {
    // Exact, not a range: a range lets npm pair mismatched builds.
    assert.equal(entry.optionalDependencies[spec.packageName], f.version);
  }
  assert.deepEqual(entry.bin, { adfc: "bin/adfc.js" });
  assert.equal(entry.preferUnplugged, true);
  fs.rmSync(f.root, { recursive: true, force: true });
});

test("all seven manifests share one version", () => {
  const f = makeFixtures();
  const out = run(f);
  const versions = new Set(
    packageDirs(out).map(
      (dir) => JSON.parse(fs.readFileSync(path.join(dir, "package.json"), "utf8")).version,
    ),
  );
  assert.deepEqual([...versions], [f.version]);
  fs.rmSync(f.root, { recursive: true, force: true });
});

test("platform packages declare os and cpu, and carry no libc constraint", () => {
  const f = makeFixtures();
  const out = run(f);
  for (const spec of PLATFORMS) {
    const m = manifest(out, spec.packageName);
    assert.deepEqual(m.os, spec.os, spec.packageName);
    assert.deepEqual(m.cpu, spec.cpu, spec.packageName);
    // Linux is a single static musl build serving glibc hosts too, so a libc
    // constraint here would wrongly exclude most installs.
    assert.ok(!("libc" in m), `${spec.packageName} should not constrain libc`);
  }
  fs.rmSync(f.root, { recursive: true, force: true });
});

test("windows packages contain adfc.exe, others contain adfc", () => {
  const f = makeFixtures();
  const out = run(f);
  for (const spec of PLATFORMS) {
    const binary = path.join(out, spec.packageName, spec.binaryName);
    assert.ok(fs.existsSync(binary), `${spec.packageName}/${spec.binaryName}`);
    assert.match(fs.readFileSync(binary, "utf8"), new RegExp(spec.rustTarget));
    assert.ok(manifest(out, spec.packageName).files.includes(spec.binaryName));
  }
  fs.rmSync(f.root, { recursive: true, force: true });
});

test("every package ships a LICENSE", () => {
  const f = makeFixtures();
  const out = run(f);
  for (const dir of packageDirs(out)) {
    assert.ok(fs.existsSync(path.join(dir, "LICENSE")), `${dir}/LICENSE`);
  }
  fs.rmSync(f.root, { recursive: true, force: true });
});

test("entry package ships the shim and its platform table", () => {
  const f = makeFixtures();
  const out = run(f);
  const entryDir = path.join(out, ENTRY_PACKAGE);
  assert.ok(fs.existsSync(path.join(entryDir, "bin", "adfc.js")));

  const table = JSON.parse(
    fs.readFileSync(path.join(entryDir, "platforms.json"), "utf8"),
  );
  assert.equal(table.length, 6);
  // The shim resolves by these fields; a build-time-only field leaking in
  // would mean the two sides disagree about the contract.
  for (const entry of table) {
    assert.deepEqual(Object.keys(entry).sort(), [
      "binaryName",
      "cpu",
      "os",
      "packageName",
    ]);
  }
  fs.rmSync(f.root, { recursive: true, force: true });
});

test("a missing archive fails loudly instead of emitting an empty package", () => {
  const f = makeFixtures();
  fs.rmSync(path.join(f.artifacts, PLATFORMS[0].archive));
  assert.throws(() => run(f), (error) => {
    assert.match(String(error.stderr), /missing release archive/);
    return true;
  });
  fs.rmSync(f.root, { recursive: true, force: true });
});

test("a plan with no adfc release is rejected", () => {
  const f = makeFixtures();
  fs.writeFileSync(f.planPath, JSON.stringify({ releases: [] }));
  assert.throws(() => run(f), (error) => {
    assert.match(String(error.stderr), /exactly one adfc version/);
    return true;
  });
  fs.rmSync(f.root, { recursive: true, force: true });
});

test("--only packages one platform but still lists all six as optional deps", () => {
  const f = makeFixtures();
  // Only the host archive exists in a local end-to-end check, so the entry
  // package must still describe the whole release.
  for (const spec of PLATFORMS.slice(1)) {
    fs.rmSync(path.join(f.artifacts, spec.archive));
  }
  const out = run(f, ["--only", PLATFORMS[0].rustTarget]);

  assert.deepEqual(
    Object.keys(byName(out)).sort(),
    [ENTRY_PACKAGE, PLATFORMS[0].packageName].sort(),
  );
  assert.equal(
    Object.keys(manifest(out, ENTRY_PACKAGE).optionalDependencies).length,
    6,
  );
  fs.rmSync(f.root, { recursive: true, force: true });
});
