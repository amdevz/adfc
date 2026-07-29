#![forbid(unsafe_code)]

//! Markdown -> Atlassian Document Format (ADF) conversion.
//!
//! The emitted document targets the official ADF JSON Schema
//! (<http://go.atlassian.com/adf-json-schema>, vendored in schema/).

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde_json::{Map, Value, json};
use std::sync::OnceLock;

/// The official Atlassian ADF JSON Schema, compiled into the binary.
///
/// Embedded rather than read from disk so a binary installed from a registry
/// can validate its own output: an `npx adfc` or `cargo install adfc` user has
/// no checkout, and a validation feature that needs a file path is unreachable
/// for them. The vendored file stays the single source of truth, so refreshing
/// it from <http://go.atlassian.com/adf-json-schema> needs no code change.
pub const ADF_SCHEMA: &str = include_str!("../schema/adf-schema.json");

/// The compiled validator for [`ADF_SCHEMA`], built once per process.
///
/// Private: `jsonschema` is a 0.x dependency, so exposing its types here would
/// make every one of its breaking releases a breaking release of this crate.
///
/// Compiling this schema costs roughly 15ms and dominates the cost of a
/// conversion, so it is cached: a caller validating many documents pays it
/// once. A single CLI run validates once and sees no benefit, but nor does it
/// pay anything extra.
///
/// # Panics
///
/// Panics if the embedded schema fails to parse or compile. That is a build
/// defect rather than a runtime condition: the schema is fixed at compile
/// time, so a failure would occur on every run and is caught by the test
/// suite, which compiles the same validator.
fn validator() -> &'static jsonschema::Validator {
    static VALIDATOR: OnceLock<jsonschema::Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        // Panicking here is correct: the schema is embedded at compile time, so
        // a failure is a build defect that every run would hit, not a runtime
        // condition a caller could handle. The test suite compiles it too, so
        // a bad vendored schema fails CI rather than reaching a user.
        let schema: Value =
            serde_json::from_str(ADF_SCHEMA).expect("vendored ADF schema is valid JSON");
        jsonschema::validator_for(&schema).expect("vendored ADF schema compiles")
    })
}

/// The schema violations found in a document, rendered one per line.
#[derive(Debug, thiserror::Error)]
#[error("{}", .0.join("\n"))]
pub struct SchemaViolations(Vec<String>);

impl SchemaViolations {
    /// The individual violations, each already carrying its instance path.
    #[must_use]
    pub fn violations(&self) -> &[String] {
        &self.0
    }
}

/// Validate a document against the embedded [`ADF_SCHEMA`].
///
/// ```
/// let doc = adfc::markdown_to_adf("# Title");
/// assert!(adfc::validate(&doc).is_ok());
/// ```
///
/// # Errors
///
/// Returns every violation found, not just the first, so one run surfaces the
/// whole problem rather than one layer of it.
pub fn validate(doc: &Value) -> Result<(), SchemaViolations> {
    validate_with(validator(), doc)
}

/// Validate a document against an arbitrary ADF schema.
///
/// Backs the CLI's `--schema` override, which checks against a newer Atlassian
/// revision than the vendored one without rebuilding. Takes the schema as a
/// [`Value`] rather than a compiled validator so no `jsonschema` type appears
/// in this crate's public API.
///
/// # Errors
///
/// [`SchemaError::InvalidSchema`] if `schema` is not usable as a JSON Schema,
/// otherwise every violation found in `doc`.
pub fn validate_against(schema: &Value, doc: &Value) -> Result<(), SchemaError> {
    let validator =
        jsonschema::validator_for(schema).map_err(|e| SchemaError::InvalidSchema(e.to_string()))?;
    validate_with(&validator, doc).map_err(SchemaError::Violations)
}

/// The failure modes of validating against a caller-supplied schema.
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    /// The supplied schema could not be compiled.
    #[error("not a usable JSON Schema: {0}")]
    InvalidSchema(String),
    /// The document violated the schema.
    #[error("{0}")]
    Violations(#[from] SchemaViolations),
}

fn validate_with(validator: &jsonschema::Validator, doc: &Value) -> Result<(), SchemaViolations> {
    let violations: Vec<String> = validator
        .iter_errors(doc)
        .map(|e| format!("{e} at {}", e.instance_path))
        .collect();
    if violations.is_empty() {
        Ok(())
    } else {
        Err(SchemaViolations(violations))
    }
}

/// URL scheme marking an image as an issue attachment rather than a remote one.
///
/// `![alt](attachment:diagram.svg)` becomes a media node whose url is left as
/// the placeholder; the tool that uploads the file rewrites it to the real
/// attachment content URL. Keeping the scheme in the emitted document means the
/// conversion stays a pure function with no network access or site credentials.
pub const ATTACHMENT_SCHEME: &str = "attachment:";

/// Convert a Markdown string into an ADF document (`{version: 1, type: "doc", ...}`).
///
/// Never fails: unrepresentable constructs degrade rather than error
/// (remote images become labeled links, raw HTML is kept as plain text).
/// Images using the [`ATTACHMENT_SCHEME`] are the exception — they become
/// real media nodes, since the uploader can resolve them.
///
/// ```
/// let doc = adfc::markdown_to_adf("# Title");
/// assert_eq!(doc["content"][0]["type"], "heading");
/// assert_eq!(doc["content"][0]["attrs"]["level"], 1);
/// ```
#[must_use]
pub fn markdown_to_adf(markdown: &str) -> Value {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(markdown, options);
    let mut builder = Builder::new();
    for event in parser {
        builder.event(event);
    }
    builder.finish()
}

/// A block container being assembled, with the ADF node kind it will become.
struct Frame {
    node_type: &'static str,
    attrs: Option<Value>,
    content: Vec<Value>,
    /// Container whose children must be block nodes: loose inline content
    /// gets wrapped into a trailing paragraph.
    wraps_inline: bool,
}

struct Builder {
    stack: Vec<Frame>,
    /// Active inline marks (strong, em, strike, link), innermost last.
    marks: Vec<Value>,
    /// Inside a table header row, cells become tableHeader instead of tableCell.
    in_table_head: bool,
    /// Alt text accumulates here while inside an image, then degrades to a link.
    image_dest: Option<String>,
    image_alt: String,
    /// Checkbox state of the list item being built, when it is a task item.
    task_state: Option<&'static str>,
    /// Counter backing the localId every taskList/taskItem must carry.
    local_id: usize,
}

/// Map a GitHub alert marker (`> [!NOTE]`) to an ADF panelType.
fn panel_type_for(marker: &str) -> Option<&'static str> {
    match marker {
        "NOTE" => Some("note"),
        "TIP" => Some("success"),
        "IMPORTANT" => Some("info"),
        "WARNING" => Some("warning"),
        "CAUTION" => Some("error"),
        _ => None,
    }
}

/// Read a `[!MARKER]` alert tag from the start of a blockquote's first text run.
fn alert_marker(text: &str) -> Option<&'static str> {
    let rest = text.trim_start().strip_prefix("[!")?;
    let marker = rest.split(']').next()?;
    panel_type_for(marker)
}

impl Builder {
    fn new() -> Self {
        Builder {
            stack: vec![Frame {
                node_type: "doc",
                attrs: None,
                content: Vec::new(),
                wraps_inline: true,
            }],
            marks: Vec::new(),
            in_table_head: false,
            image_dest: None,
            image_alt: String::new(),
            task_state: None,
            local_id: 0,
        }
    }

    /// Sequential ids are enough: they only need to be unique per document.
    fn next_local_id(&mut self) -> String {
        self.local_id += 1;
        format!("t{}", self.local_id)
    }

    fn push(&mut self, node_type: &'static str, attrs: Option<Value>, wraps_inline: bool) {
        self.stack.push(Frame {
            node_type,
            attrs,
            content: Vec::new(),
            wraps_inline,
        });
    }

    fn pop(&mut self) {
        let mut frame = self.stack.pop().expect("balanced events");

        // A list item carrying a checkbox becomes a taskItem, whose content is
        // INLINE (no paragraph wrapper) per the ADF schema.
        if frame.node_type == "listItem"
            && let Some(state) = self.task_state.take()
        {
            {
                let inline = frame
                    .content
                    .drain(..)
                    .flat_map(|node| match node {
                        Value::Object(mut obj) if obj["type"] == "paragraph" => obj
                            .remove("content")
                            .and_then(|c| match c {
                                Value::Array(items) => Some(items),
                                _ => None,
                            })
                            .unwrap_or_default(),
                        other => vec![other],
                    })
                    .collect::<Vec<_>>();
                let local_id = self.next_local_id();
                self.append_block_or_inline(json!({
                    "type": "taskItem",
                    "attrs": {"localId": local_id, "state": state},
                    "content": inline,
                }));
                return;
            }
        }

        // A list holding taskItems is a taskList, not a bullet/ordered list.
        if matches!(frame.node_type, "bulletList" | "orderedList")
            && frame
                .content
                .iter()
                .any(|child| child["type"] == "taskItem")
        {
            let local_id = self.next_local_id();
            let content = std::mem::take(&mut frame.content);
            // Any plain listItems alongside tasks would be schema-invalid
            // inside a taskList; keep only the task items.
            let tasks: Vec<Value> = content
                .into_iter()
                .filter(|child| child["type"] == "taskItem")
                .collect();
            self.append_block_or_inline(json!({
                "type": "taskList",
                "attrs": {"localId": local_id},
                "content": tasks,
            }));
            return;
        }

        if frame.node_type == "paragraph" {
            self.try_promote_alert(&mut frame);
            // Drop content-less paragraphs; keep structural nodes as-is.
            if frame.content.is_empty() {
                return;
            }
        }
        let mut node = Map::new();
        node.insert("type".into(), json!(frame.node_type));
        if let Some(attrs) = frame.attrs {
            node.insert("attrs".into(), attrs);
        }
        node.insert("content".into(), Value::Array(frame.content));
        self.append_block_or_inline(Value::Object(node));
    }

    /// Append a block-level node, hoisting it past any enclosing paragraph.
    ///
    /// ADF paragraphs accept inline content only, so a block emitted while a
    /// paragraph frame is open (an image, which the parser always reports
    /// inside one) has to become the paragraph's sibling instead of its child.
    /// A paragraph left with no content is dropped by `pop`, so an image on its
    /// own line yields the block alone rather than trailing an empty paragraph.
    fn append_hoisted_block(&mut self, node: Value) {
        let idx = self
            .stack
            .iter()
            .rposition(|frame| frame.node_type != "paragraph")
            .expect("the doc frame is never a paragraph");
        self.stack[idx].content.push(node);
    }

    /// Append a finished node to the current frame, wrapping inline nodes in a
    /// paragraph when the container requires block-level children.
    fn append_block_or_inline(&mut self, node: Value) {
        let is_inline = matches!(
            node["type"].as_str(),
            Some("text" | "hardBreak" | "emoji" | "mention")
        );
        let frame = self.stack.last_mut().expect("non-empty stack");
        if is_inline && frame.wraps_inline {
            // Append into a trailing paragraph, creating one if needed.
            let needs_new = !matches!(
                frame.content.last(),
                Some(last) if last["type"] == "paragraph"
            );
            if needs_new {
                frame
                    .content
                    .push(json!({"type": "paragraph", "content": []}));
            }
            let para = frame.content.last_mut().unwrap();
            para["content"].as_array_mut().unwrap().push(node);
        } else {
            frame.content.push(node);
        }
    }

    /// Promote `> [!NOTE]`-style blockquotes to ADF panels.
    ///
    /// Runs as the first paragraph of a blockquote closes: the parser splits
    /// `[!NOTE]` across several text events, so the marker is only detectable
    /// once the paragraph's runs are joined. On a match the enclosing
    /// blockquote is retagged and the marker text is stripped.
    fn try_promote_alert(&mut self, paragraph: &mut Frame) -> bool {
        let quote_idx = match self.stack.len().checked_sub(1) {
            Some(idx) if self.stack[idx].node_type == "blockquote" => idx,
            _ => return false,
        };
        if !self.stack[quote_idx].content.is_empty() {
            return false;
        }

        let joined: String = paragraph
            .content
            .iter()
            .filter_map(|n| n["text"].as_str())
            .collect();
        let Some(panel_type) = alert_marker(&joined) else {
            return false;
        };

        self.stack[quote_idx].node_type = "panel";
        self.stack[quote_idx].attrs = Some(json!({ "panelType": panel_type }));

        // Drop the runs making up "[!MARKER]", then any leading whitespace.
        let marker_len = joined.find(']').map_or(0, |i| i + 1);
        let mut consumed = 0usize;
        paragraph.content.retain(|node| {
            let len = node["text"].as_str().map_or(0, str::len);
            if consumed < marker_len {
                consumed += len;
                return false;
            }
            true
        });
        if let Some(first) = paragraph.content.first_mut()
            && let Some(text) = first["text"].as_str()
        {
            let trimmed = text.trim_start().to_string();
            if trimmed.is_empty() {
                paragraph.content.remove(0);
            } else {
                first["text"] = json!(trimmed);
            }
        }
        true
    }

    fn text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.image_dest.is_some() {
            self.image_alt.push_str(text);
            return;
        }
        let mut node = Map::new();
        node.insert("type".into(), json!("text"));
        node.insert("text".into(), json!(text));
        if !self.marks.is_empty() {
            node.insert("marks".into(), Value::Array(self.marks.clone()));
        }
        self.append_block_or_inline(Value::Object(node));
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            // Math extensions are not enabled, but if they ever are, the raw
            // expression text degrades to plain text like any other run.
            Event::Text(t) | Event::InlineMath(t) | Event::DisplayMath(t) => self.text(&t),
            Event::TaskListMarker(checked) => {
                self.task_state = Some(if checked { "DONE" } else { "TODO" });
            }
            Event::Code(t) => {
                // Inline code inside image alt text contributes its literal
                // text to the accumulating label, like any other text run.
                if self.image_dest.is_some() {
                    self.image_alt.push_str(&t);
                    return;
                }
                // ADF treats code as near-exclusive: alongside it a text node
                // may carry only link and annotation. Emitting the enclosing
                // strong/em/strike as well produces a node matching neither
                // code_inline_node nor formatted_text_inline_node, which the
                // API rejects. The emphasis is dropped rather than the code
                // because code is what changes the meaning of the run.
                let mut marks: Vec<Value> = self
                    .marks
                    .iter()
                    .filter(|m| m["type"] == "link")
                    .cloned()
                    .collect();
                marks.push(json!({"type": "code"}));
                let node = json!({"type": "text", "text": t.as_ref(), "marks": marks});
                self.append_block_or_inline(node);
            }
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.append_block_or_inline(json!({"type": "hardBreak"})),
            Event::Rule => {
                let frame = self.stack.last_mut().expect("non-empty stack");
                frame.content.push(json!({"type": "rule"}));
            }
            // Raw HTML has no ADF equivalent; keep it visible as plain text.
            Event::Html(t) | Event::InlineHtml(t) => self.text(t.trim_end_matches('\n')),
            Event::FootnoteReference(_) => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => self.push("paragraph", None, false),
            Tag::Heading { level, .. } => {
                let level = heading_level(level);
                self.push("heading", Some(json!({ "level": level })), false);
            }
            Tag::BlockQuote(_) => self.push("blockquote", None, true),
            Tag::CodeBlock(kind) => {
                let attrs = match kind {
                    CodeBlockKind::Fenced(lang) if !lang.is_empty() => {
                        Some(json!({"language": lang.as_ref()}))
                    }
                    _ => None,
                };
                self.push("codeBlock", attrs, false);
            }
            Tag::List(Some(start)) => {
                self.push("orderedList", Some(json!({ "order": start })), false);
            }
            Tag::List(None) => self.push("bulletList", None, false),
            Tag::Item => self.push("listItem", None, true),
            Tag::Table(_) => self.push("table", None, false),
            Tag::TableHead => {
                self.in_table_head = true;
                self.push("tableRow", None, false);
            }
            Tag::TableRow => self.push("tableRow", None, false),
            Tag::TableCell => {
                let cell = if self.in_table_head {
                    "tableHeader"
                } else {
                    "tableCell"
                };
                self.push(cell, None, true);
            }
            Tag::Emphasis => self.marks.push(json!({"type": "em"})),
            Tag::Strong => self.marks.push(json!({"type": "strong"})),
            Tag::Strikethrough => self.marks.push(json!({"type": "strike"})),
            Tag::Link { dest_url, .. } => self
                .marks
                .push(json!({"type": "link", "attrs": {"href": dest_url.as_ref()}})),
            Tag::Image { dest_url, .. } => {
                self.image_dest = Some(dest_url.to_string());
                self.image_alt.clear();
            }
            Tag::FootnoteDefinition(_)
            | Tag::HtmlBlock
            | Tag::MetadataBlock(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Superscript
            | Tag::Subscript => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::BlockQuote(_)
            | TagEnd::List(_)
            | TagEnd::Item
            | TagEnd::Table
            | TagEnd::TableRow
            | TagEnd::TableCell => self.pop(),
            TagEnd::CodeBlock => {
                // Merge the accumulated text and strip the trailing newline
                // the parser includes.
                let frame = self.stack.last_mut().expect("non-empty stack");
                let merged: String = frame
                    .content
                    .drain(..)
                    .filter_map(|n| n["text"].as_str().map(String::from))
                    .collect();
                let merged = merged.strip_suffix('\n').unwrap_or(&merged).to_string();
                if !merged.is_empty() {
                    frame.content.push(json!({"type": "text", "text": merged}));
                }
                self.pop();
            }
            TagEnd::TableHead => {
                self.in_table_head = false;
                self.pop();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                self.marks.pop();
            }
            TagEnd::Image => {
                let dest = self.image_dest.take().unwrap_or_default();
                let alt = std::mem::take(&mut self.image_alt);

                if dest.starts_with(ATTACHMENT_SCHEME) {
                    // An `external` media node, not a `file` one: a file node
                    // additionally requires a media id and collection, which
                    // Jira's REST API never exposes for an attachment. The url
                    // stays the `attachment:` placeholder for the apply step to
                    // rewrite once it has uploaded the file and knows its
                    // content URL.
                    let mut attrs = Map::new();
                    attrs.insert("type".into(), json!("external"));
                    attrs.insert("url".into(), json!(dest));
                    if !alt.is_empty() {
                        attrs.insert("alt".into(), json!(alt));
                    }
                    self.append_hoisted_block(json!({
                        "type": "mediaSingle",
                        "attrs": {"layout": "center"},
                        "content": [{"type": "media", "attrs": Value::Object(attrs)}],
                    }));
                    return;
                }

                // Every other scheme: ADF has no way to reference an image we
                // cannot upload, so degrade to a labeled link and keep the
                // reference visible.
                let label = if alt.is_empty() { dest.clone() } else { alt };
                let mut marks = self.marks.clone();
                marks.push(json!({"type": "link", "attrs": {"href": dest}}));
                let node = json!({"type": "text", "text": label, "marks": marks});
                self.append_block_or_inline(node);
            }
            TagEnd::FootnoteDefinition
            | TagEnd::HtmlBlock
            | TagEnd::MetadataBlock(_)
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::Superscript
            | TagEnd::Subscript => {}
        }
    }

    fn finish(mut self) -> Value {
        assert_eq!(self.stack.len(), 1, "unbalanced markdown events");
        let doc = self.stack.pop().unwrap();
        json!({
            "version": 1,
            "type": "doc",
            "content": doc.content,
        })
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_schema_parses() {
        let schema: Value =
            serde_json::from_str(ADF_SCHEMA).expect("embedded schema is valid JSON");
        assert_eq!(schema["$schema"], "http://json-schema.org/draft-04/schema#");
    }

    #[test]
    fn validator_is_cached() {
        // Two calls must hand back the same compiled validator: compiling the
        // schema is ~15ms and dominates a conversion, so a fresh compile per
        // call would be the whole cost of validation paid repeatedly.
        assert!(std::ptr::eq(validator(), validator()));
    }

    #[test]
    fn validate_accepts_converted_doc() {
        let doc = markdown_to_adf("# Title\n\nSome **bold** text.");
        assert!(validate(&doc).is_ok());
    }

    #[test]
    fn validate_rejects_handmade_invalid_doc() {
        // heading levels are constrained to 1..=6 by the schema.
        let doc = json!({
            "version": 1,
            "type": "doc",
            "content": [{
                "type": "heading",
                "attrs": {"level": 99},
                "content": [{"type": "text", "text": "nope"}],
            }],
        });
        assert!(validate(&doc).is_err());
    }

    #[test]
    fn violations_render_one_per_line() {
        let doc = json!({"version": 1, "type": "doc", "content": [
            {"type": "heading", "attrs": {"level": 99}, "content": [{"type": "text", "text": "a"}]},
            {"type": "heading", "attrs": {"level": 42}, "content": [{"type": "text", "text": "b"}]},
        ]});
        let err = validate(&doc).expect_err("invalid doc must fail validation");
        let rendered = err.to_string();
        assert!(rendered.lines().count() >= 2, "got: {rendered}");
    }

    #[test]
    fn violations_carry_instance_paths() {
        let doc = json!({"version": 1, "type": "doc", "content": [
            {"type": "heading", "attrs": {"level": 99}, "content": [{"type": "text", "text": "a"}]},
        ]});
        let err = validate(&doc).expect_err("invalid doc must fail validation");
        assert!(err.to_string().contains("/content/0"), "got: {err}");
    }

    #[test]
    fn empty_document_is_valid() {
        let doc = markdown_to_adf("");
        assert_eq!(doc["content"], json!([]));
        assert!(validate(&doc).is_ok());
    }
}
