# adfc

Convert Markdown to [Atlassian Document Format
(ADF)](https://developer.atlassian.com/cloud/jira/platform/apis/document/structure/),
the rich-text JSON format Atlassian's Cloud REST APIs use for rich text —
Jira issue descriptions and comments, Confluence content, and anywhere else
ADF appears.

Built for pipelines that treat Atlassian documents as rendered artifacts:
Markdown in, schema-valid ADF out, no network access, single static binary.

## Usage

```sh
adfc ticket-body.md -o description.json

# Files are optional on both ends: stdin and stdout are the defaults
adfc ticket-body.md > description.json
cat ticket-body.md | adfc | jq .
```

- Markdown in (a `FILE` argument, or stdin), compact ADF JSON
  (`{"version":1,"type":"doc",...}`) out (`-o FILE`, or stdout).
- **Output is validated against the ADF schema by default.** The schema is
  compiled into the binary, so this works with no checkout present. Violations
  are printed to stderr and the process exits non-zero **without writing any
  output**, so a malformed document fails locally instead of at the Atlassian
  API — and never reaches a downstream consumer.
- `--no-validate`: skip validation.
- `--schema <file>`: validate against a different schema revision than the
  embedded one. Conflicts with `--no-validate`.
- `--version` / `--help`.

Exit codes: `0` success (including a downstream pipe closing early), `1`
runtime failure (unreadable input, unwritable output, schema violation), `2`
usage error.

## Supported Markdown

| Markdown                          | ADF                                     |
| --------------------------------- | --------------------------------------- |
| Headings `#`..`######`            | `heading` (levels 1-6)                  |
| Paragraphs                        | `paragraph`                             |
| Bullet / ordered lists (nested)   | `bulletList` / `orderedList` (`order` preserves the start number) |
| Fenced / indented code blocks     | `codeBlock` (`language` from the fence) |
| Blockquotes                       | `blockquote`                            |
| Tables (GFM)                      | `table` / `tableRow` / `tableHeader` / `tableCell` |
| `---`                             | `rule`                                  |
| Hard breaks (trailing spaces)     | `hardBreak`                             |
| `- [ ]` / `- [x]` task lists      | `taskList` / `taskItem` (`TODO` / `DONE`) |
| `> [!NOTE]` GitHub alerts         | `panel` (see mapping below)             |
| `![alt](attachment:file.png)`     | `mediaSingle` / `media` (see images below) |
| `**bold**` `*em*` `` `code` `` `~~strike~~` `[link](url)` | `strong` / `em` / `code` / `strike` / `link` marks |

Alert-to-panel mapping (the marker line is consumed, not rendered):

| Markdown        | ADF `panelType` |
| --------------- | --------------- |
| `> [!NOTE]`     | `note`          |
| `> [!TIP]`      | `success`       |
| `> [!IMPORTANT]`| `info`          |
| `> [!WARNING]`  | `warning`       |
| `> [!CAUTION]`  | `error`         |

A blockquote without a marker stays a plain `blockquote`.

### Images

Images split on their URL scheme:

- **`attachment:` URLs** become a `mediaSingle` wrapping an `external` media
  node, so a diagram renders inline. The URL keeps its `attachment:`
  placeholder for a later upload step to rewrite once it knows the content
  URL. An `external` node is used rather than a `file` node because a `file`
  node additionally requires a media id and collection, which the REST API
  never exposes for an attachment. Media is hoisted out of any enclosing
  paragraph, since ADF paragraphs accept inline content only.
- **Every other URL** degrades to a link labeled with its alt text. ADF has
  no way to reference an image the pipeline cannot upload, so the reference
  is kept visible rather than dropped.

Other degradations (ADF has no lossless equivalent):

- **Raw HTML** is kept as plain text rather than dropped.
- Soft line breaks become spaces (matching how Atlassian renders flowed text).

## Schema

`schema/adf-schema.json` is the official Atlassian ADF JSON Schema, vendored
from <http://go.atlassian.com/adf-json-schema>. Every integration test
validates its output against it; `--schema` applies the same check at
runtime. To refresh:

```sh
curl -sSL http://go.atlassian.com/adf-json-schema > schema/adf-schema.json
```

## Development

The toolchain is pinned in `flake.lock`, so every contributor and CI build
with the same `rustc`, `rustfmt`, `clippy`, `just` and `prek`. There is
nothing to install beyond [nix](https://nixos.org/download/) with flakes
enabled.

```sh
git clone https://github.com/amdevz/adfc && cd adfc

direnv allow          # if you use direnv: the shell loads on entry
nix develop           # otherwise: enter the shell explicitly

just                  # list the available recipes
just check            # fmt-check + lint + test — the same gate CI runs
just install-hooks    # run those gates automatically on commit
```

| Recipe | Does |
| ------ | ---- |
| `just check` | Everything CI runs; the gate before pushing |
| `just test` | Test suite |
| `just lint` | clippy at pedantic, warnings denied |
| `just format` / `just fmt-check` | Format in place / fail if unformatted |
| `just build` | Release binary at `target/release/adfc` |
| `just audit` | Dependencies against the RustSec advisory database |
| `just hooks` | Full pre-commit suite, including file-hygiene hooks |
| `just run FILE` | Convert a file and pretty-print the ADF |

CI invokes these same recipes inside the same devShell, so a green `just
check` locally and a green CI run mean the same thing.

Without nix, any Rust toolchain carrying the `rustfmt` and `clippy`
components works — note that `clippy` is not part of a bare `cargo` install,
and its absence is what the hooks fail on first. You lose the version pinning
that keeps `cargo fmt --check` stable across machines.

To install the binary: `cargo install --path .`

No runtime dependencies; the release profile builds a stripped, LTO'd
binary.

## License

MIT
