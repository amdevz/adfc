# Changelog

Notable changes to this project. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

While the version is below 1.0, breaking changes may land in a minor release.

## [Unreleased]

## [0.0.1-rc.2] - 2026-08-04

No changes to conversion or the CLI.

### Security

- Releases publish over trusted publishing (OIDC) instead of long-lived registry
  tokens, so no publishing credential exists in the repository. The crates.io
  token is minted per run and revoked when the job ends, and npm authorises each
  publish against the workflow's own identity.

## [0.0.1-rc.1] - 2026-08-04

### Added

- Markdown to ADF conversion covering headings, paragraphs, nested bullet and
  ordered lists, fenced and indented code blocks, blockquotes, GFM tables,
  rules, hard breaks, task lists, and the `strong` / `em` / `code` / `strike` /
  `link` marks.
- Nesting that ADF forbids degrades instead of producing a document the API
  rejects: a heading inside a blockquote or list item keeps its prominence as
  bold text, nested quotes flatten, a task list inside a quote becomes a bullet
  list, and a table is lifted to the nearest ancestor that accepts one, after
  the container it came from. Emphasis is only applied to runs that can carry
  it, so a degraded heading containing inline code or a `status` badge stays
  valid.
- GitHub alert blockquotes (`> [!NOTE]` and friends) become ADF panels.
- `attachment:` image URLs become `mediaSingle` / `media` nodes so a diagram
  renders inline; other URLs degrade to labelled links.
- Output is validated against the official Atlassian ADF JSON Schema, which is
  compiled into the binary. Validation runs by default, `--no-validate` skips
  it, and `--schema` checks against a different revision.
- Validation is bounded at `MAX_VALIDATION_DEPTH` (128) levels of nesting,
  reported as `ValidationError::TooDeep`. The ADF schema is a recursive `anyOf`
  union, so checking cost compounds with depth: 41 KB of nested lists previously
  exhausted 2 GB and aborted the process, and now fails in milliseconds under
  6 MB. The limit matches `serde_json`'s default recursion limit, so no document
  that a default parser could read back is refused.
- Raw ADF embeds: a fenced block with the `adf` info string carries one node or
  an array of them, and a code span prefixed `adf:` carries one inline node.
  An embed is placed where it was written and never relocated. One that cannot
  be honoured fails validation with its source line and the field at fault,
  checked against the schema definition for its own node type rather than the
  document-wide union, and its text stays in the output as visible code.
- `markdown_to_adf` returns a `Conversion` rather than a bare `Value`, so an
  embed that could not be honoured travels with the document: a failed embed
  leaves a valid `codeBlock` behind, which the document alone can never reveal.
  `validate` takes that `Conversion`; `validate_document` checks an ADF document
  that came from somewhere else.
- A file argument and `-o/--output`, with stdin and stdout as the defaults.
- Prebuilt binaries for Linux, macOS and Windows on x64 and arm64, distributed
  through npm as an entry package plus per-platform packages.

[Unreleased]: https://github.com/amdevz/adfc/compare/v0.0.1-rc.2...HEAD
[0.0.1-rc.2]: https://github.com/amdevz/adfc/compare/v0.0.1-rc.1...v0.0.1-rc.2
[0.0.1-rc.1]: https://github.com/amdevz/adfc/releases/tag/v0.0.1-rc.1
