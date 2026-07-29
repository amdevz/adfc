//! Markdown -> Atlassian Document Format (ADF) conversion.
//!
//! The emitted document targets the official ADF JSON Schema
//! (http://go.atlassian.com/adf-json-schema, vendored in schema/).

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde_json::{json, Map, Value};

/// Convert a Markdown string into an ADF document (`{version: 1, type: "doc", ...}`).
///
/// Never fails: unrepresentable constructs degrade rather than error
/// (images become labeled links, raw HTML is kept as plain text).
///
/// ```
/// let doc = jira_md2adf::markdown_to_adf("# Title");
/// assert_eq!(doc["content"][0]["type"], "heading");
/// assert_eq!(doc["content"][0]["attrs"]["level"], 1);
/// ```
pub fn markdown_to_adf(markdown: &str) -> Value {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
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
        }
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
        let frame = self.stack.pop().expect("balanced events");
        // Drop content-less paragraphs; keep structural nodes as-is.
        if frame.node_type == "paragraph" && frame.content.is_empty() {
            return;
        }
        let mut node = Map::new();
        node.insert("type".into(), json!(frame.node_type));
        if let Some(attrs) = frame.attrs {
            node.insert("attrs".into(), attrs);
        }
        node.insert("content".into(), Value::Array(frame.content));
        self.append_block_or_inline(Value::Object(node));
    }

    /// Append a finished node to the current frame, wrapping inline nodes in a
    /// paragraph when the container requires block-level children.
    fn append_block_or_inline(&mut self, node: Value) {
        let is_inline = matches!(
            node["type"].as_str(),
            Some("text") | Some("hardBreak") | Some("emoji") | Some("mention")
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
            Event::Text(t) => self.text(&t),
            Event::Code(t) => {
                // Inline code inside image alt text contributes its literal
                // text to the accumulating label, like any other text run.
                if self.image_dest.is_some() {
                    self.image_alt.push_str(&t);
                    return;
                }
                let mut marks = self.marks.clone();
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
            Event::FootnoteReference(_) | Event::TaskListMarker(_) => {}
            Event::InlineMath(t) | Event::DisplayMath(t) => self.text(&t),
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => self.push("paragraph", None, false),
            Tag::Heading { level, .. } => {
                let level = heading_level(level);
                self.push("heading", Some(json!({ "level": level })), false)
            }
            Tag::BlockQuote(_) => self.push("blockquote", None, true),
            Tag::CodeBlock(kind) => {
                let attrs = match kind {
                    CodeBlockKind::Fenced(lang) if !lang.is_empty() => {
                        Some(json!({"language": lang.as_ref()}))
                    }
                    _ => None,
                };
                self.push("codeBlock", attrs, false)
            }
            Tag::List(Some(start)) => {
                self.push("orderedList", Some(json!({ "order": start })), false)
            }
            Tag::List(None) => self.push("bulletList", None, false),
            Tag::Item => self.push("listItem", None, true),
            Tag::Table(_) => self.push("table", None, false),
            Tag::TableHead => {
                self.in_table_head = true;
                self.push("tableRow", None, false)
            }
            Tag::TableRow => self.push("tableRow", None, false),
            Tag::TableCell => {
                let cell = if self.in_table_head {
                    "tableHeader"
                } else {
                    "tableCell"
                };
                self.push(cell, None, true)
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
                self.pop()
            }
            TagEnd::TableHead => {
                self.in_table_head = false;
                self.pop()
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                self.marks.pop();
            }
            TagEnd::Image => {
                // ADF media nodes need uploaded attachment ids; degrade to a
                // labeled link so the reference survives.
                let dest = self.image_dest.take().unwrap_or_default();
                let label = if self.image_alt.is_empty() {
                    dest.clone()
                } else {
                    std::mem::take(&mut self.image_alt)
                };
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
