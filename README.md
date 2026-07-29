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
adfc < ticket-body.md > description.json

# Validate the output against the official ADF JSON Schema before printing
adfc --schema schema/adf-schema.json < ticket-body.md
```

- Markdown on stdin, compact ADF JSON (`{"version":1,"type":"doc",...}`) on
  stdout.
- `--schema <file>`: validate the emitted document against an ADF JSON
  Schema (draft-04). Violations are printed to stderr and the process exits
  non-zero, so malformed documents fail locally instead of at the Atlassian
  API.
- `--version` / `--help`.

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

## Build

```sh
cargo test             # 26 integration tests + doctests
cargo build --release  # target/release/adfc
cargo install --path . # install into ~/.cargo/bin
```

No runtime dependencies; the release profile builds a stripped, LTO'd
binary.

## License

MIT
