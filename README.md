# adfc

Convert Markdown to [Atlassian Document Format
(ADF)](https://developer.atlassian.com/cloud/jira/platform/apis/document/structure/) —
the JSON that Atlassian Cloud REST APIs accept for rich text, in Jira issues
and comments, Confluence content, and anywhere else ADF appears.

Markdown in, schema-valid ADF out. No network access, no configuration.

## Install

**As a command:**

```sh
npx @amdevz/adfc --version   # without installing
npm i -g @amdevz/adfc        # globally
npm i -D @amdevz/adfc        # as a project dev dependency
cargo install adfc           # from crates.io
```

The npm package is scoped, but the command it installs is `adfc`.

The npm packages ship a prebuilt binary for Linux, macOS and Windows on x64 and
arm64. Nothing is compiled or fetched during install, so `npm ci
--ignore-scripts`, offline caches and mirrored registries all work. Linux is a
static musl build and runs on Alpine as well as glibc.

**As a library:**

```sh
cargo add adfc --no-default-features
```

`--no-default-features` drops the CLI's argument-parsing dependencies, which the
library does not use.

## Usage

```sh
adfc ticket.md -o description.json
adfc ticket.md > description.json     # stdout is the default
cat ticket.md | adfc | jq .           # stdin too
```

| Flag | Effect |
| --- | --- |
| `-o, --output <FILE>` | Write here instead of stdout |
| `--no-validate` | Skip schema validation |
| `--schema <FILE>` | Validate against a different ADF schema revision |
| `-h, --help` / `-V, --version` | |

Output is validated against the ADF schema by default. The schema is compiled
into the binary, so this needs no files on disk. On a violation, every error
goes to stderr and **nothing is written** — a malformed document fails here
rather than at the Atlassian API, and never reaches a consumer.

Validation is bounded at 128 levels of nesting. The ADF schema is a recursive
union of `anyOf` branches, so the cost of checking compounds with depth; past
this point a document is refused rather than checked, naming its own depth. The
bound matches `serde_json`'s default recursion limit, so nothing that could be
parsed back is turned away. `--no-validate` converts such a document anyway.

Exit codes: `0` success, including a downstream pipe closing early; `1` runtime
failure; `2` usage error.

## Library

```rust
let converted = adfc::markdown_to_adf("# Title\n\nSome **bold** text.");
adfc::validate(&converted)?;

// The document is a serde_json::Value, ready to PUT as an issue description.
println!("{}", converted.doc());
```

`markdown_to_adf` cannot fail: constructs with no ADF equivalent degrade rather
than error. It returns a `Conversion` rather than the document alone, so
anything the conversion could not honour travels with the document and
`validate` can refuse it. `validate` checks a conversion against the embedded
schema, `validate_against` checks it against one you supply, and
`validate_document` checks an ADF document that came from somewhere else.

## Supported Markdown

| Markdown | ADF |
| --- | --- |
| `#`..`######` | `heading`, levels 1-6 |
| Paragraphs | `paragraph` |
| Bullet and ordered lists, nested | `bulletList` / `orderedList`, preserving the start number |
| Fenced and indented code blocks | `codeBlock`, with the fence's language |
| Blockquotes | `blockquote` |
| GFM tables | `table` / `tableRow` / `tableHeader` / `tableCell` |
| `---` | `rule` |
| Hard breaks | `hardBreak` |
| `- [ ]` / `- [x]` | `taskList` / `taskItem` |
| `> [!NOTE]` alerts | `panel` |
| `![alt](attachment:f.png)` | `mediaSingle` / `media` |
| `**bold**` `*em*` `` `code` `` `~~strike~~` `[link](url)` | `strong` / `em` / `code` / `strike` / `link` |

GitHub alerts map to panel types: `NOTE` → `note`, `TIP` → `success`,
`IMPORTANT` → `info`, `WARNING` → `warning`, `CAUTION` → `error`. A blockquote
without a marker stays a `blockquote`.

## Raw ADF embeds

Markdown cannot spell every ADF node. Where it cannot, write the node itself.

A fenced block with the `adf` info string carries one node, or an array of
them, and becomes those nodes in place:

````markdown
```adf
{"type": "status", "attrs": {"text": "Done", "color": "green"}}
```
````

A code span prefixed `adf:` carries exactly one inline node, so a badge can sit
inside a sentence:

```markdown
The build is `adf:{"type":"status","attrs":{"text":"Done","color":"green"}}`.
```

An embed is placed where it was written and never relocated: Markdown-authored
content hoists out of a container ADF forbids it in, because Markdown cannot
express ADF's nesting rules, but an embed names its node explicitly and moving
it would change what was asked for. A node its container forbids is refused
instead.

An embed that cannot be honoured — malformed JSON, an unknown node type, a node
that violates its own schema definition, or a block node in an inline span —
fails the run, naming the source line and the field at fault. The text stays in
the output as visible code, so nothing the author wrote is lost, and
`--no-validate` emits that form rather than failing.

## Where ADF cannot follow

- **Images** with an `attachment:` URL become media nodes, keeping the scheme as
  a placeholder for an uploader to rewrite. Every other URL degrades to a link
  labelled with its alt text, since ADF cannot reference an image this tool
  cannot upload.
- **Inline code** loses any surrounding emphasis; ADF forbids combining them.
- **Raw HTML** is kept as plain text.
- **Soft line breaks** become spaces, matching how Atlassian renders flowed text.

## Contributing

The toolchain is pinned in `flake.lock`, so a clone plus
[nix](https://nixos.org/download/) is the whole setup:

```sh
direnv allow    # or: nix develop
just check      # format, lint, both test suites
```

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT — see [LICENSE](LICENSE).
