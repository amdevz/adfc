//! ADF -> HTML, in the dialect Atlassian's Rovo MCP server accepts as
//! `contentFormat: "html"`.
//!
//! The dialect has no public specification: the server serves it at runtime
//! through `getContentFormatGuide`. Each element maps to one ADF node
//! (`<div data-type="panel-info">` is a panel), so this module is a mechanical
//! walk of an already-validated tree. docs/html-dialect.md records the mapping
//! and the guide revision it was taken from.
//!
//! Anything without a documented form is refused by name and path rather than
//! approximated: the server would otherwise store something other than what
//! the author wrote, and nothing downstream would say so.

use crate::{ATTACHMENT_SCHEME, MAX_VALIDATION_DEPTH, nesting_depth};
use serde_json::Value;
use std::fmt::Write as _;

/// Why a document could not be rendered as HTML.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HtmlError {
    /// Nodes, marks or nesting the dialect has no form for, one per line,
    /// each with the instance path it was found at.
    #[error("{}", .0.join("\n"))]
    Unsupported(Vec<String>),
    /// Nesting beyond [`MAX_VALIDATION_DEPTH`], refused before rendering for
    /// the same reason validation refuses it.
    #[error("document nests {depth} levels deep, over the limit of {limit}")]
    TooDeep {
        /// The document's actual nesting depth.
        depth: usize,
        /// The limit that was exceeded, always [`MAX_VALIDATION_DEPTH`].
        limit: usize,
    },
}

/// Node types the guide excludes from a panel although the schema admits
/// them: "Not table, expand, blockquote, embed card, or nested panel", with
/// tables kept out "for editor compatibility".
const PANEL_FORBIDS: &[&str] = &["table", "expand", "blockquote", "embedCard", "panel"];

/// Render an ADF document in the HTML dialect Atlassian's Rovo MCP server
/// accepts as `contentFormat: "html"`.
///
/// Takes any ADF document, not only one from [`crate::markdown_to_adf`], so a
/// document fetched from Jira can be rendered too. Validate it first: this
/// checks only what the dialect needs, not the ADF schema.
///
/// The output is a body fragment with no `<html>` or `<body>` wrapper, and
/// targets what Jira and Confluence both accept. Confluence-only nodes
/// (layouts, extensions, sync blocks) are refused.
///
/// ```
/// let converted = adfc::markdown_to_adf("> [!NOTE]\n> Read **this**.");
/// adfc::validate(&converted).unwrap();
/// let html = adfc::adf_to_html(converted.doc()).unwrap();
/// assert_eq!(html, "<div data-type=\"panel-note\"><p>Read <strong>this</strong>.</p></div>");
/// ```
///
/// # Errors
///
/// [`HtmlError::TooDeep`] if the document nests beyond
/// [`MAX_VALIDATION_DEPTH`], checked first. Otherwise
/// [`HtmlError::Unsupported`] listing every node, mark or nesting without an
/// HTML form, not just the first.
pub fn adf_to_html(doc: &Value) -> Result<String, HtmlError> {
    // Bounded before the walk, which recurses: with the bound in place the
    // recursion cannot outgrow the stack, whatever the input.
    let depth = nesting_depth(doc);
    if depth > MAX_VALIDATION_DEPTH {
        return Err(HtmlError::TooDeep {
            depth,
            limit: MAX_VALIDATION_DEPTH,
        });
    }
    if doc["type"] != "doc" {
        return Err(HtmlError::Unsupported(vec![format!(
            "expected a doc at the root, found {}",
            type_name(doc)
        )]));
    }

    let mut renderer = Renderer::default();
    renderer.blocks(doc, "");
    if renderer.problems.is_empty() {
        Ok(renderer.out)
    } else {
        Err(HtmlError::Unsupported(renderer.problems))
    }
}

#[derive(Default)]
struct Renderer {
    out: String,
    problems: Vec<String>,
}

impl Renderer {
    /// Render each child of `parent`, which sits at `path`.
    fn blocks(&mut self, parent: &Value, path: &str) {
        for (i, child) in children(parent).iter().enumerate() {
            self.block(child, &format!("{path}/content/{i}"));
        }
    }

    fn inlines(&mut self, parent: &Value, path: &str) {
        for (i, child) in children(parent).iter().enumerate() {
            self.inline(child, &format!("{path}/content/{i}"));
        }
    }

    /// A node type with no form at all.
    fn no_form(&mut self, node: &Value, path: &str) {
        self.problems.push(format!(
            "{} at {path} has no HTML form the server accepts",
            type_name(node)
        ));
    }

    /// A mapped node type whose particular attributes cannot be expressed.
    fn invalid(&mut self, node: &Value, path: &str, why: &str) {
        self.problems
            .push(format!("{} at {path}: {why}", type_name(node)));
    }

    fn no_mark_form(&mut self, mark: &str, path: &str) {
        self.problems.push(format!(
            "{mark} mark at {path} has no HTML form the server accepts"
        ));
    }

    /// Open `tag`, render the children with `body`, close it.
    fn wrap(&mut self, tag: &str, attrs: &str, body: impl FnOnce(&mut Self)) {
        let _ = write!(self.out, "<{tag}{attrs}>");
        body(self);
        let _ = write!(self.out, "</{tag}>");
    }

    #[allow(clippy::too_many_lines)] // One arm per node type reads as the mapping table it is.
    fn block(&mut self, node: &Value, path: &str) {
        let attrs = &node["attrs"];
        match node["type"].as_str().unwrap_or_default() {
            "paragraph" => {
                let style = self.block_style(node, path);
                self.wrap("p", &style, |r| r.inlines(node, path));
            }
            "heading" => match attrs["level"].as_u64() {
                Some(level @ 1..=6) => {
                    let style = self.block_style(node, path);
                    self.wrap(&format!("h{level}"), &style, |r| r.inlines(node, path));
                }
                _ => self.invalid(node, path, "level must be 1 to 6"),
            },
            "bulletList" => self.wrap("ul", "", |r| r.blocks(node, path)),
            "orderedList" => {
                let start = attrs["order"].as_u64().unwrap_or(1);
                self.wrap("ol", &format!(" start=\"{start}\""), |r| {
                    r.blocks(node, path);
                });
            }
            "listItem" => self.wrap("li", "", |r| r.blocks(node, path)),
            "blockquote" => self.wrap("blockquote", "", |r| r.blocks(node, path)),
            "tableHeader" => self.wrap("th", &cell_attrs(attrs), |r| r.blocks(node, path)),
            "tableCell" => self.wrap("td", &cell_attrs(attrs), |r| r.blocks(node, path)),
            "table" => self.wrap("table", &table_attrs(attrs), |r| r.blocks(node, path)),
            "tableRow" => self.wrap("tr", "", |r| r.blocks(node, path)),
            "taskList" => {
                let attrs = format!(" data-type=\"task-list\"{}", local_id(attrs));
                self.wrap("ul", &attrs, |r| r.blocks(node, path));
            }
            "taskItem" | "blockTaskItem" => {
                let block_item = node["type"] == "blockTaskItem";
                let kind = if block_item {
                    "block-task-item"
                } else {
                    "task-item"
                };
                let checked = if attrs["state"] == "DONE" {
                    " checked"
                } else {
                    ""
                };
                let attrs = format!(" data-type=\"{kind}\"{}", local_id(attrs));
                self.wrap("li", &attrs, |r| {
                    let _ = write!(r.out, "<input type=\"checkbox\"{checked}>");
                    if block_item {
                        r.blocks(node, path);
                    } else {
                        r.out.push(' ');
                        r.inlines(node, path);
                    }
                });
            }
            "decisionList" => {
                self.wrap("ul", " data-type=\"decision-list\"", |r| {
                    r.blocks(node, path);
                });
            }
            "decisionItem" => {
                let state = attrs["state"].as_str().unwrap_or("UNDECIDED");
                let attrs = format!(
                    " data-type=\"decision-item\" data-state=\"{}\"",
                    escape(state)
                );
                self.wrap("li", &attrs, |r| r.inlines(node, path));
            }
            "codeBlock" => {
                let breakout = self.breakout(node, path);
                let class = attrs["language"]
                    .as_str()
                    .filter(|lang| !lang.is_empty())
                    .map(|lang| format!(" class=\"language-{}\"", escape(lang)))
                    .unwrap_or_default();
                let text: String = children(node)
                    .iter()
                    .filter_map(|run| run["text"].as_str())
                    .collect();
                let _ = write!(
                    self.out,
                    "<pre{breakout}><code{class}>{}</code></pre>",
                    escape(&text)
                );
            }
            "panel" => self.panel(node, path),
            "rule" => self.out.push_str("<hr>"),
            "expand" | "nestedExpand" => {
                let mut attrs = if node["type"] == "nestedExpand" {
                    String::from(" data-type=\"nested-expand\"")
                } else {
                    String::new()
                };
                attrs.push_str(&self.breakout(node, path));
                let title = escape(node["attrs"]["title"].as_str().unwrap_or_default());
                self.wrap("details", &attrs, |r| {
                    let _ = write!(r.out, "<summary>{title}</summary>");
                    r.blocks(node, path);
                });
            }
            // The canonical form rather than the `data-card-appearance`
            // shorthand: it is what the server echoes back, so a fetched body
            // written back unchanged stays byte-stable.
            "blockCard" => match attrs["url"].as_str() {
                Some(url) => {
                    let url = escape(url);
                    let _ = write!(
                        self.out,
                        "<div data-type=\"block-card\" data-url=\"{url}\"><a href=\"{url}\">{url}</a></div>"
                    );
                }
                None => self.invalid(node, path, "only a card with a url has an HTML form"),
            },
            "embedCard" => match attrs["url"].as_str() {
                Some(url) => {
                    let mut div = String::from(" data-type=\"embed-card\"");
                    let layout = attrs["layout"].as_str().unwrap_or("center");
                    let _ = write!(div, " data-layout=\"{}\"", escape(layout));
                    push_number(&mut div, "data-width", &attrs["width"]);
                    push_number(&mut div, "data-original-width", &attrs["originalWidth"]);
                    push_number(&mut div, "data-original-height", &attrs["originalHeight"]);
                    let _ = write!(
                        self.out,
                        "<div{div}><iframe src=\"{}\"></iframe></div>",
                        escape(url)
                    );
                }
                None => self.invalid(node, path, "has no url"),
            },
            "mediaSingle" => self.media_single(node, path),
            "mediaGroup" => self.wrap("div", " data-type=\"media-group\"", |r| {
                for (i, media) in children(node).iter().enumerate() {
                    r.media(media, &format!("{path}/content/{i}"));
                }
            }),
            "media" => self.media(node, path),
            "caption" => self.wrap("figcaption", "", |r| r.inlines(node, path)),
            _ => self.no_form(node, path),
        }
    }

    fn panel(&mut self, node: &Value, path: &str) {
        let attrs = &node["attrs"];
        let open = match attrs["panelType"].as_str() {
            Some(kind @ ("info" | "note" | "warning" | "success" | "error")) => {
                format!(" data-type=\"panel-{kind}\"")
            }
            Some("custom") => {
                let mut open = String::from(" data-type=\"panel-custom\"");
                for (key, attr) in [
                    ("panelIcon", "data-icon"),
                    ("panelIconId", "data-icon-id"),
                    ("panelIconText", "data-icon-text"),
                    ("panelColor", "data-color"),
                ] {
                    if let Some(value) = attrs[key].as_str() {
                        let _ = write!(open, " {attr}=\"{}\"", escape(value));
                    }
                }
                open
            }
            // The server accepts `panel-tip` but stores it as an info panel,
            // so emitting it would hand back a different document unannounced.
            Some("tip") => {
                self.invalid(
                    node,
                    path,
                    "panelType tip is stored as info by the server, so it has no lossless form",
                );
                return;
            }
            _ => {
                self.invalid(node, path, "unknown panelType");
                return;
            }
        };
        for (i, child) in children(node).iter().enumerate() {
            let child_type = type_name(child);
            if PANEL_FORBIDS.contains(&child_type) {
                self.problems.push(format!(
                    "{child_type} inside panel at {path}/content/{i}: the server's guide forbids this nesting"
                ));
            }
        }
        self.wrap("div", &open, |r| r.blocks(node, path));
    }

    /// An image or file. An external image needs no upload; a file needs the
    /// media id and collection an upload returns.
    fn media_single(&mut self, node: &Value, path: &str) {
        let parts = children(node);
        let Some(media) = parts.first() else {
            self.invalid(node, path, "has no media");
            return;
        };
        let caption = parts.get(1);
        let media_path = format!("{path}/content/0");
        let attrs = &media["attrs"];

        if attrs["type"] == "external" && caption.is_none() {
            self.external_image(media, &media_path);
            return;
        }

        let outer = &node["attrs"];
        let mut figure = String::from(" data-type=\"media-single\"");
        let layout = outer["layout"].as_str().unwrap_or("center");
        let _ = write!(figure, " data-layout=\"{}\"", escape(layout));
        push_number(&mut figure, "data-width", &outer["width"]);
        if let Some(width_type) = outer["widthType"].as_str() {
            let _ = write!(figure, " data-width-type=\"{}\"", escape(width_type));
        }
        self.wrap("figure", &figure, |r| {
            if attrs["type"] == "external" {
                r.external_image(media, &media_path);
            } else {
                r.media(media, &media_path);
            }
            if let Some(caption) = caption {
                r.block(caption, &format!("{path}/content/1"));
            }
        });
    }

    fn external_image(&mut self, media: &Value, path: &str) {
        let attrs = &media["attrs"];
        let url = attrs["url"].as_str().unwrap_or_default();
        if url.starts_with(ATTACHMENT_SCHEME) {
            // Rendering it would publish a broken image; the placeholder only
            // means something to an uploader working on the ADF.
            self.problems.push(format!(
                "image {url} at {path} must be uploaded first; embed it by media id"
            ));
            return;
        }
        let mut img = format!(" src=\"{}\"", escape(url));
        if let Some(alt) = attrs["alt"].as_str() {
            let _ = write!(img, " alt=\"{}\"", escape(alt));
        }
        push_number(&mut img, "width", &attrs["width"]);
        push_number(&mut img, "height", &attrs["height"]);
        let _ = write!(self.out, "<img{img}>");
    }

    /// A file or link media node, by the id and collection an upload returned.
    fn media(&mut self, node: &Value, path: &str) {
        let attrs = &node["attrs"];
        if attrs["type"] == "external" {
            self.external_image(node, path);
            return;
        }
        let (Some(kind), Some(id)) = (attrs["type"].as_str(), attrs["id"].as_str()) else {
            self.invalid(node, path, "needs a type and the id an upload returned");
            return;
        };
        let mut div = format!(
            " data-type=\"media\" data-media-type=\"{}\" data-id=\"{}\"",
            escape(kind),
            escape(id)
        );
        for (key, attr) in [
            ("collection", "data-collection"),
            ("alt", "data-alt"),
            ("occurrenceKey", "data-occurrence-key"),
        ] {
            if let Some(value) = attrs[key].as_str() {
                let _ = write!(div, " {attr}=\"{}\"", escape(value));
            }
        }
        push_number(&mut div, "data-width", &attrs["width"]);
        push_number(&mut div, "data-height", &attrs["height"]);
        let _ = write!(self.out, "<div{div}></div>");
    }

    /// A smart card from its URL. The `data` and `datasource` variants carry
    /// resolved content or a query, which the dialect cannot express.
    fn inline_card(&mut self, node: &Value, path: &str) {
        match node["attrs"]["url"].as_str() {
            Some(url) => {
                let _ = write!(
                    self.out,
                    "<a href=\"{}\" data-card-appearance=\"inline\"></a>",
                    escape(url)
                );
            }
            None => self.invalid(node, path, "only a card with a url has an HTML form"),
        }
    }

    /// An inline node, wrapped in any inline-comment anchors it carries.
    fn inline(&mut self, node: &Value, path: &str) {
        let anchors: Vec<&Value> = marks(node)
            .iter()
            .filter(|mark| mark["type"] == "annotation")
            .collect();
        // An annotation anchors the whole run, link and styling included, so
        // it is outermost.
        for anchor in &anchors {
            let attrs = &anchor["attrs"];
            let _ = write!(
                self.out,
                "<span class=\"annotation\" data-annotation-id=\"{}\" data-annotation-type=\"{}\">",
                escape(attrs["id"].as_str().unwrap_or_default()),
                escape(attrs["annotationType"].as_str().unwrap_or("inlineComment"))
            );
        }
        self.inline_body(node, path);
        for _ in &anchors {
            self.out.push_str("</span>");
        }
    }

    #[allow(clippy::too_many_lines)] // One arm per node type, as in `block`.
    fn inline_body(&mut self, node: &Value, path: &str) {
        let attrs = &node["attrs"];
        let text_of = |key: &str| escape(attrs[key].as_str().unwrap_or_default());
        match node["type"].as_str().unwrap_or_default() {
            "text" => self.text(node, path),
            "hardBreak" => self.out.push_str("<br>"),
            "status" => {
                let mut span = format!(" data-type=\"status\" data-color=\"{}\"", text_of("color"));
                if let Some(style) = attrs["style"].as_str().filter(|s| !s.is_empty()) {
                    let _ = write!(span, " data-status-style=\"{}\"", escape(style));
                }
                let _ = write!(self.out, "<span{span}>{}</span>", text_of("text"));
            }
            "emoji" => {
                let short_name = text_of("shortName");
                let mut span = format!(" data-type=\"emoji\" data-shortname=\"{short_name}\"");
                // Both or neither, as the guide requires: with neither the
                // server fills them from the shortname.
                let shown = match (attrs["id"].as_str(), attrs["text"].as_str()) {
                    (Some(id), Some(text)) => {
                        let _ = write!(
                            span,
                            " data-emoji-id=\"{}\" data-emoji-text=\"{}\"",
                            escape(id),
                            escape(text)
                        );
                        escape(text)
                    }
                    _ => short_name,
                };
                let _ = write!(self.out, "<span{span}>{shown}</span>");
            }
            "date" => {
                let timestamp = attrs["timestamp"].as_str().unwrap_or_default();
                if let Ok(millis) = timestamp.parse::<i64>() {
                    let day = calendar_date(millis);
                    let _ = write!(self.out, "<time datetime=\"{day}\">{day}</time>");
                } else {
                    self.invalid(
                        node,
                        path,
                        &format!("timestamp {timestamp:?} is not milliseconds since the epoch"),
                    );
                }
            }
            "mention" => {
                let Some(id) = attrs["id"].as_str() else {
                    self.invalid(node, path, "has no account id");
                    return;
                };
                let mut span = format!(" data-type=\"mention\" data-user-id=\"{}\"", escape(id));
                if let Some(level) = attrs["accessLevel"].as_str() {
                    let _ = write!(span, " data-access-level=\"{}\"", escape(level));
                }
                if let Some(kind) = attrs["userType"].as_str() {
                    let _ = write!(span, " data-user-type=\"{}\"", escape(kind));
                }
                let label = attrs["text"].as_str().unwrap_or("@");
                let _ = write!(self.out, "<span{span}>{}</span>", escape(label));
            }
            "placeholder" => {
                let _ = write!(
                    self.out,
                    "<span data-type=\"placeholder\">{}</span>",
                    text_of("text")
                );
            }
            "inlineCard" => self.inline_card(node, path),
            "mediaInline" => {
                let mut span = String::from(" data-type=\"media-inline\"");
                for (key, attr) in [
                    ("type", "data-media-type"),
                    ("id", "data-media-id"),
                    ("collection", "data-media-collection"),
                    ("alt", "data-alt"),
                    ("occurrenceKey", "data-occurrence-key"),
                ] {
                    if let Some(value) = attrs[key].as_str() {
                        let _ = write!(span, " {attr}=\"{}\"", escape(value));
                    }
                }
                push_number(&mut span, "data-width", &attrs["width"]);
                push_number(&mut span, "data-height", &attrs["height"]);
                let _ = write!(self.out, "<span{span}></span>");
            }
            _ => self.no_form(node, path),
        }
    }

    fn text(&mut self, node: &Value, path: &str) {
        let mut link = None;
        let mut has_code = false;
        // (opening tag, closing tag), outermost first.
        let mut wrappers: Vec<(String, &str)> = Vec::new();
        for mark in marks(node) {
            let attrs = &mark["attrs"];
            let color = || escape(attrs["color"].as_str().unwrap_or_default());
            match mark["type"].as_str().unwrap_or_default() {
                "link" => link = attrs["href"].as_str(),
                // Rendered around the whole run by `inline`.
                "annotation" => {}
                "code" => has_code = true,
                "strong" => wrappers.push(("<strong>".into(), "</strong>")),
                "em" => wrappers.push(("<em>".into(), "</em>")),
                "strike" => wrappers.push(("<s>".into(), "</s>")),
                "underline" => wrappers.push(("<u>".into(), "</u>")),
                "subsup" if attrs["type"] == "sub" => {
                    wrappers.push(("<sub>".into(), "</sub>"));
                }
                "subsup" => wrappers.push(("<sup>".into(), "</sup>")),
                "textColor" => {
                    wrappers.push((format!("<span style=\"color: {}\">", color()), "</span>"));
                }
                "backgroundColor" => wrappers.push((
                    format!("<span style=\"background-color: {}\">", color()),
                    "</span>",
                )),
                other => self.no_mark_form(other, path),
            }
        }
        // The dialect keeps the link and drops code styling when both apply,
        // so emitting `<code>` inside `<a>` would promise what it discards.
        if has_code && link.is_none() {
            wrappers.push(("<code>".into(), "</code>"));
        }

        // The link is outermost so a run carrying other marks stays one link.
        if let Some(href) = link {
            let _ = write!(self.out, "<a href=\"{}\">", escape(href));
        }
        for (open, _) in &wrappers {
            self.out.push_str(open);
        }
        self.out
            .push_str(&escape(node["text"].as_str().unwrap_or_default()));
        for (_, close) in wrappers.iter().rev() {
            self.out.push_str(close);
        }
        if link.is_some() {
            self.out.push_str("</a>");
        }
    }

    /// The `style` attribute for a paragraph or heading's block marks.
    fn block_style(&mut self, node: &Value, path: &str) -> String {
        let mut styles: Vec<String> = Vec::new();
        for mark in marks(node) {
            let attrs = &mark["attrs"];
            match mark["type"].as_str().unwrap_or_default() {
                "alignment" => styles.push(format!(
                    "text-align: {}",
                    escape(attrs["align"].as_str().unwrap_or_default())
                )),
                "indentation" => {
                    let level = attrs["level"].as_u64().unwrap_or(1);
                    styles.push(format!("margin-left: {}px", level * 30));
                }
                // ADF's only size; the guide maps anything up to 14px to it.
                "fontSize" => styles.push("font-size: 14px".into()),
                other => self.no_mark_form(other, path),
            }
        }
        if styles.is_empty() {
            String::new()
        } else {
            format!(" style=\"{}\"", styles.join("; "))
        }
    }

    /// `data-breakout` for a code block or expand carrying a breakout mark.
    fn breakout(&mut self, node: &Value, path: &str) -> String {
        let mut out = String::new();
        for mark in marks(node) {
            if mark["type"] == "breakout" {
                let attrs = &mark["attrs"];
                let mode = attrs["mode"].as_str().unwrap_or("wide");
                let _ = write!(out, " data-breakout=\"{}\"", escape(mode));
                push_number(&mut out, "data-breakout-width", &attrs["width"]);
            } else {
                self.no_mark_form(type_name(mark), path);
            }
        }
        out
    }
}

/// A node's children, or none.
fn children(node: &Value) -> &[Value] {
    node["content"].as_array().map_or(&[], Vec::as_slice)
}

fn marks(node: &Value) -> &[Value] {
    node["marks"].as_array().map_or(&[], Vec::as_slice)
}

fn type_name(node: &Value) -> &str {
    node["type"].as_str().unwrap_or("a node without a type")
}

/// Append ` name="n"` for a numeric attribute, when present.
///
/// Integers print without a fraction (`80`, not `80.0`); the guide wants
/// unitless finite decimals and drops anything else.
fn push_number(out: &mut String, name: &str, value: &Value) {
    if let Some(n) = value.as_u64() {
        let _ = write!(out, " {name}=\"{n}\"");
    } else if let Some(n) = value.as_f64().filter(|n| n.is_finite()) {
        let _ = write!(out, " {name}=\"{n}\"");
    }
}

/// `data-local-id`, kept only when it is a server-issued id.
///
/// The dialect wants ids omitted on new nodes (the server generates them) and
/// 12 lowercase hex characters otherwise. adfc's own `t1`, `t2` are neither,
/// while an id on a fetched document is, and dropping that would detach
/// anything targeting the node.
fn local_id(attrs: &Value) -> String {
    match attrs["localId"].as_str() {
        Some(id)
            if id.len() == 12 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) =>
        {
            format!(" data-local-id=\"{id}\"")
        }
        _ => String::new(),
    }
}

/// Table attributes a fetched table may carry. adfc never sets them, but
/// dropping them from a fetched table would reset its layout on write-back.
fn table_attrs(attrs: &Value) -> String {
    let mut out = String::new();
    if attrs["isNumberColumnEnabled"] == true {
        out.push_str(" data-number-column=\"true\"");
    }
    for (key, attr) in [
        ("layout", "data-layout"),
        ("displayMode", "data-display-mode"),
    ] {
        if let Some(value) = attrs[key].as_str() {
            let _ = write!(out, " {attr}=\"{}\"", escape(value));
        }
    }
    push_number(&mut out, "data-width", &attrs["width"]);
    out
}

/// Cell attributes, for the same reason as [`table_attrs`].
fn cell_attrs(attrs: &Value) -> String {
    let mut out = String::new();
    for key in ["colspan", "rowspan"] {
        push_number(&mut out, key, &attrs[key]);
    }
    if let Some(widths) = attrs["colwidth"].as_array() {
        let joined: Vec<String> = widths.iter().map(ToString::to_string).collect();
        let _ = write!(out, " data-colwidth=\"{}\"", joined.join(","));
    }
    if let Some(background) = attrs["background"].as_str() {
        let _ = write!(out, " data-background=\"{}\"", escape(background));
    }
    out
}

/// The UTC calendar day of a millisecond timestamp, as `YYYY-MM-DD`.
///
/// Howard Hinnant's days-to-civil algorithm, exact over the proleptic
/// Gregorian calendar; a date dependency for one conversion would outweigh it.
fn calendar_date(millis: i64) -> String {
    let days = millis.div_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Escape text and attribute values alike: one function, so an attribute can
/// never be written with the weaker text escaping.
fn escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Node types with no form in the Jira-safe dialect, refused on purpose.
    /// Extensions and sync blocks are Confluence macros; Jira refuses layouts
    /// outright ("Jira saves an empty field when a body contains one").
    const REFUSED_NODES: &[&str] = &[
        "bodiedExtension",
        "bodiedSyncBlock",
        "extension",
        "inlineExtension",
        "layoutColumn",
        "layoutSection",
        "syncBlock",
    ];

    /// Marks the guide documents no form for.
    const REFUSED_MARKS: &[&str] = &["border", "dataConsumer", "fragment"];

    /// Every `type` the vendored schema declares under a definition suffix.
    fn schema_types(suffix: &str) -> Vec<String> {
        let mut types: Vec<String> = crate::definitions()
            .as_object()
            .expect("definitions is an object")
            .iter()
            .filter(|(name, _)| name.ends_with(suffix))
            .filter_map(|(_, definition)| {
                definition["properties"]["type"]["enum"][0]
                    .as_str()
                    .map(String::from)
            })
            .collect();
        types.sort();
        types.dedup();
        types
    }

    fn render_error(doc: &Value) -> String {
        adf_to_html(doc)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default()
    }

    #[test]
    fn every_schema_node_type_is_mapped_or_refused_on_purpose() {
        // A schema refresh that adds a node type fails here until the type is
        // either given a form or added to the refusal list with a reason.
        let types = schema_types("_node");
        assert!(types.len() > 40, "schema read: {types:?}");
        for node_type in types.iter().filter(|t| *t != "doc") {
            let node = json!({"type": node_type});
            let content = if crate::is_inline(node_type) {
                json!([{"type": "paragraph", "content": [node]}])
            } else {
                json!([node])
            };
            let message = render_error(&json!({"version": 1, "type": "doc", "content": content}));
            let refused = message.contains(&format!("{node_type} at /"))
                && message.contains("has no HTML form");
            assert_eq!(
                refused,
                REFUSED_NODES.contains(&node_type.as_str()),
                "{node_type}: {message}"
            );
        }
    }

    #[test]
    fn every_schema_mark_is_mapped_or_refused_on_purpose() {
        let types = schema_types("_mark");
        assert!(types.len() > 15, "schema read: {types:?}");
        for mark in &types {
            let carrier = match mark.as_str() {
                "alignment" | "indentation" | "fontSize" => json!({
                    "type": "paragraph", "marks": [{"type": mark}], "content": [],
                }),
                "breakout" => json!({"type": "codeBlock", "marks": [{"type": mark}]}),
                _ => json!({"type": "paragraph", "content": [
                    {"type": "text", "text": "x", "marks": [{"type": mark}]},
                ]}),
            };
            let message = render_error(&json!({"version": 1, "type": "doc", "content": [carrier]}));
            let refused = message.contains(&format!("{mark} mark at"))
                && message.contains("has no HTML form");
            assert_eq!(
                refused,
                REFUSED_MARKS.contains(&mark.as_str()),
                "{mark}: {message}"
            );
        }
    }

    #[test]
    fn calendar_dates_across_boundaries() {
        assert_eq!(calendar_date(0), "1970-01-01");
        assert_eq!(calendar_date(951_782_400_000), "2000-02-29");
        assert_eq!(calendar_date(1_741_996_800_000), "2025-03-15");
        assert_eq!(calendar_date(1_741_996_800_000 + 86_399_999), "2025-03-15");
        assert_eq!(calendar_date(-1), "1969-12-31");
    }
}
