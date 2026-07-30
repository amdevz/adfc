# Contributing to adfc

## Setup

The toolchain is pinned in `flake.lock`, so [nix](https://nixos.org/download/)
with flakes enabled is the whole setup:

```sh
git clone https://github.com/amdevz/adfc && cd adfc
direnv allow          # or: nix develop
just check            # the gate: format, lint, both test suites
just install-hooks    # run that gate on commit
```

Without nix you need a Rust toolchain including `clippy` and `rustfmt`,
[`just`](https://github.com/casey/just),
[`prek`](https://github.com/j178/prek) (or `pre-commit`), and Node 18+. You
give up the pinning that keeps `cargo fmt --check` stable across toolchains.

## Recipes

`just` lists them all. The ones that matter: `check` (everything CI runs),
`test`, `test-node`, `lint`, `format`, `build`, `audit`, `run FILE`.

CI invokes these same recipes in the same devShell, and so do the pre-commit
hooks, so a green `just check` locally means a green CI run.

## Conventions

- **Commits** follow [Conventional Commits](https://www.conventionalcommits.org/).
  Scopes are component names (`adf`, `cli`, `npm`), never issue numbers. Keep
  each commit self-contained and passing its own tests.
- **Comments** explain why, not what. State the constraint or the failure the
  code prevents.
- **Errors** use `thiserror` in the library, `anyhow` with `.context()` in the
  binary. `unwrap()` and `expect()` are for tests and static initialisers only.
- **Tests** go in `#[cfg(test)] mod tests` beside the code, or in `tests/` —
  `adf.rs` for conversion (every emitted document is validated against the
  vendored schema), `cli.rs` for the real binary. Node tests use `node:test`,
  so there is no `package.json` or `node_modules` here.

`tests/fixtures/` holds a deliberately restrictive schema and a deliberately
malformed one, to drive paths normal input cannot reach. Do not add production
flags whose only purpose is to make a test easier.

## Refreshing the ADF schema

`schema/adf-schema.json` is vendored from Atlassian and compiled into the
binary:

```sh
curl -sSL http://go.atlassian.com/adf-json-schema > schema/adf-schema.json
just check
```

Containment rules, inline node types and per-node validators are all derived
from this file, so a revision needs no code change — and the suite fails
immediately if one disagrees with the converter.

## Distribution

Releases are cut from tags. `cargo-dist` builds six targets and
`scripts/build-npm-packages.js` turns those archives into seven npm packages
under the `@amdevz` scope: an entry package plus six platform packages in its
`optionalDependencies`. The binary ships *inside* each tarball rather than
being downloaded on install, which is what keeps `npm ci --ignore-scripts`,
offline caches and mirrored registries working.

`scripts/platforms.js` is the single Rust-target-to-npm-package mapping.
`.github/workflows/release.yml` is generated — change `dist-workspace.toml` and
run `dist generate`.

## Pull requests

Run `just check`. Add a line to `CHANGELOG.md` under *Unreleased* if the change
is user-visible; release notes are generated from it. Say what changed and why,
and name any tradeoff you made rather than leaving it for review to find.
