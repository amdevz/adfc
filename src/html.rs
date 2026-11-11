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
    /// Nodes or marks the dialect has no form for, one per line, each with the
    /// instance path it was found at.
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

/// Render an ADF document in the HTML dialect Atlassian's Rovo MCP server
/// accepts as `contentFormat: "html"`.
///
/// Takes any ADF document, not only one from [`crate::markdown_to_adf`], so a
/// document fetched from Jira can be rendered too. Validate it first: this
/// checks only what the dialect needs, not the ADF schema.
///
/// The output is a body fragment with no `<html>` or `<body>` wrapper.
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
/// [`HtmlError::Unsupported`] listing every node or mark without an HTML form,
/// not just the first.
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

    fn unsupported(&mut self, node: &Value, path: &str) {
        self.problems.push(format!(
            "{} at {path} has no HTML form the server accepts",
            type_name(node)
        ));
    }

    /// Open `tag`, render the children with `body`, close it.
    fn wrap(&mut self, tag: &str, attrs: &str, body: impl FnOnce(&mut Self)) {
        let _ = write!(self.out, "<{tag}{attrs}>");
        body(self);
        let _ = write!(self.out, "</{tag}>");
    }

    fn block(&mut self, node: &Value, path: &str) {
        let attrs = &node["attrs"];
        match node["type"].as_str().unwrap_or_default() {
            "paragraph" => self.wrap("p", "", |r| r.inlines(node, path)),
            "heading" => match attrs["level"].as_u64() {
                Some(level @ 1..=6) => {
                    self.wrap(&format!("h{level}"), "", |r| r.inlines(node, path));
                }
                _ => self.unsupported(node, path),
            },
            "bulletList" => self.wrap("ul", "", |r| r.blocks(node, path)),
            "orderedList" => {
                let start = attrs["order"].as_u64().unwrap_or(1);
                self.wrap("ol", &format!(" start=\"{start}\""), |r| {
                    r.blocks(node, path);
                });
            }
            "listItem" | "tableHeader" | "tableCell" | "blockquote" => {
                let tag = match node["type"].as_str() {
                    Some("listItem") => "li",
                    Some("tableHeader") => "th",
                    Some("tableCell") => "td",
                    _ => "blockquote",
                };
                let attrs = if tag == "th" || tag == "td" {
                    cell_attrs(attrs)
                } else {
                    String::new()
                };
                self.wrap(tag, &attrs, |r| r.blocks(node, path));
            }
            "taskList" => {
                let attrs = format!(" data-type=\"task-list\"{}", local_id(attrs));
                self.wrap("ul", &attrs, |r| r.blocks(node, path));
            }
            "taskItem" => {
                let checked = if attrs["state"] == "DONE" {
                    " checked"
                } else {
                    ""
                };
                let attrs = format!(" data-type=\"task-item\"{}", local_id(attrs));
                self.wrap("li", &attrs, |r| {
                    let _ = write!(r.out, "<input type=\"checkbox\"{checked}> ");
                    r.inlines(node, path);
                });
            }
            "table" => self.wrap("table", &table_attrs(attrs), |r| r.blocks(node, path)),
            "tableRow" => self.wrap("tr", "", |r| r.blocks(node, path)),
            "codeBlock" => {
                let class = attrs["language"]
                    .as_str()
                    .filter(|lang| !lang.is_empty())
                    .map(|lang| format!(" class=\"language-{}\"", escape(lang)))
                    .unwrap_or_default();
                let text: String = children(node)
                    .iter()
                    .filter_map(|run| run["text"].as_str())
                    .collect();
                let _ = write!(self.out, "<pre><code{class}>{}</code></pre>", escape(&text));
            }
            "panel" => match attrs["panelType"].as_str() {
                Some(kind @ ("info" | "note" | "warning" | "success" | "error")) => {
                    let attrs = format!(" data-type=\"panel-{kind}\"");
                    self.wrap("div", &attrs, |r| r.blocks(node, path));
                }
                _ => self.unsupported(node, path),
            },
            "rule" => self.out.push_str("<hr>"),
            "mediaSingle" => self.media_single(node, path),
            _ => self.unsupported(node, path),
        }
    }

    /// An image. Only an external one has a form without an upload: a file
    /// image needs the media id and collection an upload returns.
    fn media_single(&mut self, node: &Value, path: &str) {
        let media = children(node);
        let [media] = media else {
            self.unsupported(node, path);
            return;
        };
        let attrs = &media["attrs"];
        let url = attrs["url"].as_str().unwrap_or_default();
        if media["type"] != "media" || attrs["type"] != "external" {
            self.unsupported(node, path);
        } else if url.starts_with(ATTACHMENT_SCHEME) {
            // Rendering it would publish a broken image; the placeholder only
            // means something to an uploader working on the ADF.
            self.problems.push(format!(
                "image {url} at {path} must be uploaded first; embed it by media id"
            ));
        } else {
            let alt = attrs["alt"]
                .as_str()
                .map(|alt| format!(" alt=\"{}\"", escape(alt)))
                .unwrap_or_default();
            let _ = write!(self.out, "<img src=\"{}\"{alt}>", escape(url));
        }
    }

    fn inline(&mut self, node: &Value, path: &str) {
        match node["type"].as_str().unwrap_or_default() {
            "text" => self.text(node, path),
            "hardBreak" => self.out.push_str("<br>"),
            _ => self.unsupported(node, path),
        }
    }

    fn text(&mut self, node: &Value, path: &str) {
        let marks = node["marks"].as_array().map_or(&[][..], Vec::as_slice);
        let mut link = None;
        let mut has_code = false;
        let mut styles: Vec<&str> = Vec::new();
        for mark in marks {
            match mark["type"].as_str().unwrap_or_default() {
                "link" => link = mark["attrs"]["href"].as_str(),
                "strong" => styles.push("strong"),
                "em" => styles.push("em"),
                "strike" => styles.push("s"),
                "code" => has_code = true,
                other => self.problems.push(format!(
                    "{other} mark at {path} has no HTML form the server accepts"
                )),
            }
        }
        // The dialect keeps the link and drops code styling when both apply,
        // so emitting `<code>` inside `<a>` would promise what it discards.
        if has_code && link.is_none() {
            styles.push("code");
        }

        // The link is outermost so a run carrying other marks stays one link.
        if let Some(href) = link {
            let _ = write!(self.out, "<a href=\"{}\">", escape(href));
        }
        for tag in &styles {
            let _ = write!(self.out, "<{tag}>");
        }
        self.out
            .push_str(&escape(node["text"].as_str().unwrap_or_default()));
        for tag in styles.iter().rev() {
            let _ = write!(self.out, "</{tag}>");
        }
        if link.is_some() {
            self.out.push_str("</a>");
        }
    }
}

/// A node's children, or none.
fn children(node: &Value) -> &[Value] {
    node["content"].as_array().map_or(&[], Vec::as_slice)
}

fn type_name(node: &Value) -> &str {
    node["type"].as_str().unwrap_or("a node without a type")
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
    if let Some(width) = attrs["width"].as_f64() {
        let _ = write!(out, " data-width=\"{width}\"");
    }
    out
}

/// Cell attributes, for the same reason as [`table_attrs`].
fn cell_attrs(attrs: &Value) -> String {
    let mut out = String::new();
    for key in ["colspan", "rowspan"] {
        if let Some(span) = attrs[key].as_u64() {
            let _ = write!(out, " {key}=\"{span}\"");
        }
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
