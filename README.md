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

Exit codes: `0` success, including a downstream pipe closing early; `1` runtime
failure; `2` usage error.

## Library

```rust
let doc = adfc::markdown_to_adf("# Title\n\nSome **bold** text.");
adfc::validate(&doc)?;

// doc is a serde_json::Value, ready to PUT as an issue description.
println!("{doc}");
```

`markdown_to_adf` cannot fail: constructs with no ADF equivalent degrade rather
than error. `validate` checks a document against the embedded schema, and
`validate_against` checks it against one you supply.

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
