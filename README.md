# jira-md2adf

Convert Markdown to [Atlassian Document Format
(ADF)](https://developer.atlassian.com/cloud/jira/platform/apis/document/structure/),
the rich-text JSON format Jira's REST v3 API expects for descriptions and
comments.

Built for pipelines that treat Jira tickets as rendered artifacts: Markdown
in, schema-valid ADF out, no network access, single static binary.

## Usage

```sh
jira-md2adf < ticket-body.md > description.json

# Validate the output against the official ADF JSON Schema before printing
jira-md2adf --schema schema/adf-schema.json < ticket-body.md
```

- Markdown on stdin, compact ADF JSON (`{"version":1,"type":"doc",...}`) on
  stdout.
- `--schema <file>`: validate the emitted document against an ADF JSON
  Schema (draft-04). Violations are printed to stderr and the process exits
  non-zero, so malformed documents fail locally instead of at the Jira API.
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
| `**bold**` `*em*` `` `code` `` `~~strike~~` `[link](url)` | `strong` / `em` / `code` / `strike` / `link` marks |

Degradations (ADF has no lossless equivalent):

- **Images** become links labeled with their alt text — ADF `media` nodes
  require uploaded attachment ids, which a text pipeline does not have.
- **Raw HTML** is kept as plain text rather than dropped.
- Soft line breaks become spaces (matching how Jira renders flowed text).

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
cargo test             # 17 integration tests + doctests
cargo build --release  # target/release/jira-md2adf
cargo install --path . # install into ~/.cargo/bin
```

No runtime dependencies; the release profile builds a stripped, LTO'd
binary.

## License

MIT
