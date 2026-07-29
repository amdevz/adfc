# adfc

Convert Markdown to [Atlassian Document Format
(ADF)](https://developer.atlassian.com/cloud/jira/platform/apis/document/structure/),
the rich-text JSON format Atlassian's Cloud REST APIs use for rich text —
Jira issue descriptions and comments, Confluence content, and anywhere else
ADF appears.

Built for pipelines that treat Atlassian documents as rendered artifacts:
Markdown in, schema-valid ADF out, no network access, single static binary.

## Install

```sh
cargo install --path .   # from a clone
```

Prebuilt binaries and `npx adfc` arrive with the first tagged release.

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

The official Atlassian ADF JSON Schema is vendored from
<http://go.atlassian.com/adf-json-schema> and compiled into the binary, which
is why validation needs no files on disk. `--schema` swaps in a different
revision without rebuilding, for testing against a newer one than the release
carries.

## Contributing

The toolchain is pinned in `flake.lock`, so a clone plus
[nix](https://nixos.org/download/) is the whole setup:

```sh
direnv allow    # or: nix develop
just check      # format, lint, and both test suites
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the recipes, testing conventions and
release process.

## License

MIT — see [LICENSE](LICENSE).
