# Changelog

Notable changes to this project. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

While the version is below 1.0, breaking changes may land in a minor release.

## [Unreleased]

### Added

- Markdown to ADF conversion covering headings, paragraphs, nested bullet and
  ordered lists, fenced and indented code blocks, blockquotes, GFM tables,
  rules, hard breaks, task lists, and the `strong` / `em` / `code` / `strike` /
  `link` marks.
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
- A file argument and `-o/--output`, with stdin and stdout as the defaults.
- Prebuilt binaries for Linux, macOS and Windows on x64 and arm64, distributed
  through npm as an entry package plus per-platform packages.

[Unreleased]: https://github.com/amdevz/adfc/commits/main
