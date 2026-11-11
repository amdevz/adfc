# The Rovo MCP HTML dialect

`adfc --format html` and `adfc::adf_to_html` render ADF in the HTML dialect
Atlassian's Rovo MCP server accepts as `contentFormat: "html"`. This file
records what that dialect is, where it comes from, and the revision of it the
renderer in `src/html.rs` was written against.

## Source

There is no public specification, schema or version. The server describes the
dialect at runtime, to authenticated clients, through the MCP operation
`getContentFormatGuide`, which cites its source as
`@atlassian/atlassian-html-format:html-format.md`. That package is not on the
public npm registry.

The guide is one shared reference with a product preamble:

- Every Jira tool (`createJiraIssue`, `editJiraIssue`,
  `addOrEditJiraIssueComment`) returns the same text.
- Confluence (`createConfluencePage`) returns a superset: extensions (macros),
  native tabs, sync blocks and embed workflows.
- Jira refuses column layouts with an error, because it would otherwise save an
  empty field.

Because the guide is not published, its text is not vendored here. The mapping
below is written in this project's words, and the fingerprints let a
maintainer tell whether the guide has changed since.

## Revision

Fetched 2026-11-11. SHA-256 of the `body` string each call returned, as UTF-8:

| `toolName` | SHA-256 |
| --- | --- |
| `createJiraIssue`, `editJiraIssue`, `addOrEditJiraIssueComment` | `5b11d07e752ccb2976c24901648ecc32620819e20c2e9a647b5aa1dbbcc06fdc` |
| `createConfluencePage` | `365ce95a01c7b1cbd2ee462b97f525b2a56821f960e013b0054f746d54581984` |

CONTRIBUTING.md describes how to refresh these and what to do when one changes.

## Target

The renderer emits the Jira-safe core: what both the Jira and the Confluence
guides accept. Confluence-only nodes are refused rather than emitted, so a
document rendered once can be written to either product.

## Mapping

### Blocks

| ADF | HTML |
| --- | --- |
| `paragraph` | `<p>` |
| `heading` (`level` n) | `<hn>` |
| `bulletList` / `orderedList` (`order`) | `<ul>` / `<ol start="n">` |
| `listItem` | `<li>` |
| `taskList` | `<ul data-type="task-list">` |
| `taskItem` (`state`) | `<li data-type="task-item"><input type="checkbox" checked> text</li>` |
| `blockTaskItem` | `<li data-type="block-task-item"><input type="checkbox">blocks</li>` |
| `decisionList` / `decisionItem` (`state`) | `<ul data-type="decision-list">` / `<li data-type="decision-item" data-state>` |
| `table` / `tableRow` | `<table>` / `<tr>` |
| `tableHeader` / `tableCell` | `<th>` / `<td>`, with `colspan`, `rowspan`, `data-colwidth`, `data-background` |
| `codeBlock` (`language`) | `<pre><code class="language-x">` |
| `blockquote` | `<blockquote>` |
| `panel` (`panelType`) | `<div data-type="panel-TYPE">`; `custom` adds `data-icon`, `data-icon-id`, `data-icon-text`, `data-color` |
| `rule` | `<hr>` |
| `expand` / `nestedExpand` (`title`) | `<details>` / `<details data-type="nested-expand">` with `<summary>` |
| `blockCard` (`url`) | `<div data-type="block-card" data-url><a href>url</a></div>`, the form the server echoes |
| `embedCard` (`url`) | `<div data-type="embed-card" data-layout><iframe src></iframe></div>` |
| `mediaSingle` > external `media` | `<img src alt>`; inside `<figure>` when captioned |
| `mediaSingle` > file `media` | `<figure data-type="media-single" data-layout data-width data-width-type>` |
| `mediaGroup` | `<div data-type="media-group">` |
| `media` (file) | `<div data-type="media" data-media-type data-id data-collection data-alt>` |
| `caption` | `<figcaption>` |

### Inline nodes

| ADF | HTML |
| --- | --- |
| `text` | escaped text |
| `hardBreak` | `<br>` |
| `status` | `<span data-type="status" data-color data-status-style>` |
| `emoji` | `<span data-type="emoji" data-shortname data-emoji-id data-emoji-text>`; id and text both or neither |
| `date` (`timestamp`, ms) | `<time datetime="YYYY-MM-DD">`, the UTC day |
| `mention` | `<span data-type="mention" data-user-id data-access-level data-user-type>` |
| `placeholder` | `<span data-type="placeholder">` |
| `inlineCard` (`url`) | `<a href data-card-appearance="inline"></a>` |
| `mediaInline` | `<span data-type="media-inline" data-media-type data-media-id data-media-collection>` |

### Marks

| ADF | HTML |
| --- | --- |
| `strong` / `em` / `strike` / `underline` | `<strong>` / `<em>` / `<s>` / `<u>` |
| `code` | `<code>`, dropped inside a link (see below) |
| `subsup` | `<sub>` / `<sup>` |
| `link` | `<a href>`, outermost of the text marks |
| `textColor` / `backgroundColor` | `<span style="color: #x">` / `<span style="background-color: #x">` |
| `annotation` | `<span class="annotation" data-annotation-id data-annotation-type>`, outermost of all |
| `alignment` | `style="text-align: center|end"` on `<p>` or `<hn>` |
| `indentation` (`level`) | `style="margin-left: level*30px"` on `<p>` or `<hn>` |
| `fontSize` (`small`) | `style="font-size: 14px"` on `<p>` |
| `breakout` | `data-breakout` and `data-breakout-width` on `<pre>` or `<details>` |

## Rules beyond the mapping

- **Local ids.** `data-local-id` is written only for a 12-character lowercase
  hex id, the form the server issues. adfc's own `t1`, `t2` are omitted so the
  server generates its own.
- **Code in a link.** The guide keeps the link and drops code styling when a
  run has both. The renderer emits the link alone rather than a `<code>` the
  server would discard.
- **Nesting inside a panel.** The guide excludes tables, expands, blockquotes,
  embed cards and nested panels from a panel, though the schema admits a table.
  Each is refused with its path.
- **Escaping.** Text and attribute values are escaped with one function
  (`& < > " '`), so no value can open a tag or close an attribute.

## Verified against the server

Round-tripped on 2026-11-11 by posting `adfc --format html` output as a Jira
comment and reading it back as HTML. Every node and mark came back as the same
node, except as noted. Not round-tripped, because they need real upload or
annotation ids: uploaded media (`mediaSingle`/`mediaGroup`/`mediaInline` with a
file), inline-comment anchors, and the `breakout` mark.

The server fills in, and these are not drift:

- `data-local-id` and `data-unique-id` on lists, items and code blocks;
- `<thead>` / `<tbody>` around table rows, and dropping `start="1"`;
- a `date`'s display text (`2025-03-15` comes back as `March 15, 2025`, with
  the same `datetime`);
- a `mention`'s display name, and an inline card's URL as its anchor text;
- `&#39;` as `&#039;`.

Observed and unexplained: a `backgroundColor` run (`<span
style="background-color: #f8e6a0">`) came back wrapped in an added
`<span style="color: #f8e6a0">`, a text colour equal to the highlight. adfc
sends the shape the guide documents, so this is on the server side; whether it
is stored that way or only echoed that way cannot be told from the MCP
response.

## Refused

Refused by type and path rather than approximated:

| What | Why |
| --- | --- |
| `extension`, `bodiedExtension`, `inlineExtension` | Confluence macros; no Jira form |
| `layoutSection`, `layoutColumn` | Jira refuses layouts outright |
| `syncBlock`, `bodiedSyncBlock` | Confluence-only live content |
| `border`, `dataConsumer`, `fragment` marks | The guide documents no form |
| `panelType: tip` | The server stores it as `info`; no lossless form |
| `attachment:` images | No placeholder form; upload first and embed by media id |
| Cards without a `url` | The `data` and `datasource` variants have no form |

`src/html.rs` checks every node type and mark in the vendored ADF schema
against this split, so a schema refresh that adds a type fails the suite until
the type is mapped or refused here on purpose.
