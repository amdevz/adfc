//! ADF -> HTML rendering, in the dialect Atlassian's Rovo MCP server accepts as
//! `contentFormat: "html"` (see docs/html-dialect.md).
//!
//! Inputs come from `markdown_to_adf` wherever the converter can produce them,
//! so the renderer is tested on the documents it will actually receive, and
//! every one of them is validated as ADF first.

use adfc::{HtmlError, adf_to_html, markdown_to_adf};
use serde_json::{Value, json};

/// Convert, validate as ADF, then render.
fn html(md: &str) -> String {
    let converted = markdown_to_adf(md);
    if let Err(e) = adfc::validate(&converted) {
        panic!("fixture is not valid ADF: {e}");
    }
    adf_to_html(converted.doc()).unwrap_or_else(|e| panic!("render failed: {e}"))
}

fn doc(content: &Value) -> Value {
    json!({"version": 1, "type": "doc", "content": content})
}

#[test]
fn paragraph_and_headings() {
    assert_eq!(html("hello"), "<p>hello</p>");
    assert_eq!(html("# one\n\n###### six"), "<h1>one</h1><h6>six</h6>");
}

#[test]
fn inline_marks() {
    assert_eq!(
        html("**b** *i* ~~s~~ `c` [t](https://x.com)"),
        "<p><strong>b</strong> <em>i</em> <s>s</s> <code>c</code> <a href=\"https://x.com\">t</a></p>"
    );
}

#[test]
fn nested_marks_close_in_order() {
    assert_eq!(
        html("**bold *and italic***"),
        "<p><strong>bold </strong><strong><em>and italic</em></strong></p>"
    );
}

#[test]
fn link_is_the_outermost_mark() {
    // A run inside a link carrying other marks: the anchor wraps the styling,
    // so the converter sees one link rather than splitting it.
    assert_eq!(
        html("[**bold link**](https://x.com)"),
        "<p><a href=\"https://x.com\"><strong>bold link</strong></a></p>"
    );
}

#[test]
fn code_inside_a_link_keeps_the_link() {
    // The dialect cannot hold both: `<a>` keeps the link and drops code
    // styling. Emitting the code would lose nothing visible, but nesting
    // `<code>` in `<a>` is what the server documents as dropped, so the
    // output says plainly what survives.
    assert_eq!(
        html("[`x`](https://x.com)"),
        "<p><a href=\"https://x.com\">x</a></p>"
    );
}

#[test]
fn hard_break() {
    assert_eq!(html("one  \ntwo"), "<p>one<br>two</p>");
}

#[test]
fn lists() {
    assert_eq!(
        html("- a\n- b"),
        "<ul><li><p>a</p></li><li><p>b</p></li></ul>"
    );
    assert_eq!(
        html("3. a\n4. b"),
        "<ol start=\"3\"><li><p>a</p></li><li><p>b</p></li></ol>"
    );
    assert_eq!(
        html("1. a\n   - b"),
        "<ol start=\"1\"><li><p>a</p><ul><li><p>b</p></li></ul></li></ol>"
    );
}

#[test]
fn task_list_omits_generated_local_ids() {
    // adfc numbers localIds `t1`, `t2`. The dialect wants them omitted on new
    // nodes (the server generates its own), and 12 hex characters otherwise.
    assert_eq!(
        html("- [x] done\n- [ ] open"),
        "<ul data-type=\"task-list\">\
         <li data-type=\"task-item\"><input type=\"checkbox\" checked> done</li>\
         <li data-type=\"task-item\"><input type=\"checkbox\"> open</li></ul>"
    );
}

#[test]
fn well_formed_local_ids_survive() {
    // A fetched document carries server-issued ids; dropping them would detach
    // anything that targets the node.
    let fetched = doc(&json!([{
        "type": "taskList",
        "attrs": {"localId": "0a3fae7b4e9b"},
        "content": [{
            "type": "taskItem",
            "attrs": {"localId": "7394e3a237ca", "state": "TODO"},
            "content": [{"type": "text", "text": "keep"}],
        }],
    }]));
    assert_eq!(
        adf_to_html(&fetched).unwrap(),
        "<ul data-type=\"task-list\" data-local-id=\"0a3fae7b4e9b\">\
         <li data-type=\"task-item\" data-local-id=\"7394e3a237ca\"><input type=\"checkbox\"> keep</li></ul>"
    );
}

#[test]
fn table_with_header_and_empty_cell() {
    assert_eq!(
        html("| a | b |\n| --- | --- |\n| 1 | |"),
        "<table><tr><th><p>a</p></th><th><p>b</p></th></tr>\
         <tr><td><p>1</p></td><td><p></p></td></tr></table>"
    );
}

#[test]
fn code_block_with_and_without_language() {
    assert_eq!(
        html("```rust\nfn main() {}\n```"),
        "<pre><code class=\"language-rust\">fn main() {}</code></pre>"
    );
    assert_eq!(html("    plain"), "<pre><code>plain</code></pre>");
}

#[test]
fn blockquote_panel_and_rule() {
    assert_eq!(html("> quoted"), "<blockquote><p>quoted</p></blockquote>");
    assert_eq!(
        html("> [!WARNING]\n> careful"),
        "<div data-type=\"panel-warning\"><p>careful</p></div>"
    );
    assert_eq!(html("---"), "<hr>");
}

#[test]
fn every_panel_type_maps() {
    for (marker, panel) in [
        ("NOTE", "note"),
        ("TIP", "success"),
        ("IMPORTANT", "info"),
        ("WARNING", "warning"),
        ("CAUTION", "error"),
    ] {
        assert_eq!(
            html(&format!("> [!{marker}]\n> x")),
            format!("<div data-type=\"panel-{panel}\"><p>x</p></div>")
        );
    }
}

#[test]
fn text_and_attributes_are_escaped() {
    assert_eq!(
        html("a < b & c > \"d\" 'e'"),
        "<p>a &lt; b &amp; c &gt; &quot;d&quot; &#39;e&#39;</p>"
    );
    assert_eq!(
        html("[t](https://x.com/?a=1&b=\"2\")"),
        "<p><a href=\"https://x.com/?a=1&amp;b=&quot;2&quot;\">t</a></p>"
    );
    assert_eq!(
        html("```\n<script>alert(1)</script>\n```"),
        "<pre><code>&lt;script&gt;alert(1)&lt;/script&gt;</code></pre>"
    );
}

#[test]
fn raw_html_in_markdown_stays_text() {
    assert_eq!(html("<b>x</b>"), "<p>&lt;b&gt;x&lt;/b&gt;</p>");
}

#[test]
fn code_block_language_cannot_break_the_attribute() {
    assert_eq!(
        html("```a\"b\nx\n```"),
        "<pre><code class=\"language-a&quot;b\">x</code></pre>"
    );
}

#[test]
fn empty_document_renders_empty() {
    assert_eq!(html(""), "");
}

#[test]
fn attachment_image_is_refused_with_its_path() {
    // The dialect has no placeholder: a file image needs a media id and
    // collection from an upload. `<img src="attachment:x">` would publish a
    // broken image, so it is refused instead.
    let converted = markdown_to_adf("![diagram](attachment:d.png)");
    let err = adf_to_html(converted.doc()).expect_err("attachment must be refused");
    let HtmlError::Unsupported(items) = &err else {
        panic!("expected Unsupported, got {err:?}");
    };
    assert_eq!(items.len(), 1, "{items:?}");
    assert!(items[0].contains("attachment:d.png"), "{}", items[0]);
    assert!(items[0].contains("/content/0"), "{}", items[0]);
}

#[test]
fn external_image_renders_as_img() {
    let external = doc(&json!([{
        "type": "mediaSingle",
        "attrs": {"layout": "center"},
        "content": [{"type": "media", "attrs": {
            "type": "external", "url": "https://x.com/a.png", "alt": "a & b",
        }}],
    }]));
    assert_eq!(
        adf_to_html(&external).unwrap(),
        "<img src=\"https://x.com/a.png\" alt=\"a &amp; b\">"
    );
}

#[test]
fn unmapped_node_types_are_refused_together() {
    // Every problem is reported, as validation does, so one run shows the
    // author everything to change.
    let embedded = doc(&json!([
        {"type": "paragraph", "content": [
            {"type": "status", "attrs": {"text": "OK", "color": "green"}},
        ]},
        {"type": "extension", "attrs": {"extensionKey": "k", "extensionType": "t"}},
    ]));
    let err = adf_to_html(&embedded).expect_err("unmapped nodes must be refused");
    let HtmlError::Unsupported(items) = &err else {
        panic!("expected Unsupported, got {err:?}");
    };
    assert_eq!(items.len(), 2, "{items:?}");
    assert!(items[0].contains("status") && items[0].contains("/content/0/content/0"));
    assert!(items[1].contains("extension") && items[1].contains("/content/1"));
}

#[test]
fn unmapped_marks_are_refused() {
    let marked = doc(&json!([{"type": "paragraph", "content": [
        {"type": "text", "text": "x", "marks": [{"type": "textColor", "attrs": {"color": "#ff5630"}}]},
    ]}]));
    let err = adf_to_html(&marked).expect_err("unmapped mark must be refused");
    assert!(err.to_string().contains("textColor"), "{err}");
}

#[test]
fn a_non_document_is_refused() {
    let err = adf_to_html(&json!({"type": "paragraph"})).expect_err("not a doc");
    assert!(err.to_string().contains("doc"), "{err}");
}

#[test]
fn excessive_depth_is_refused_before_rendering() {
    // The same bound validation applies, for the same reason: the walk must
    // not be the thing that falls over on hostile input.
    let mut node = json!({"type": "paragraph", "content": [{"type": "text", "text": "x"}]});
    for _ in 0..200 {
        node = json!({"type": "blockquote", "content": [node]});
    }
    let err = adf_to_html(&doc(&json!([node]))).expect_err("too deep");
    assert!(matches!(err, HtmlError::TooDeep { .. }), "{err:?}");
}

#[test]
fn every_markdown_fixture_in_the_readme_renders() {
    // The README's supported-Markdown list, end to end: anything the converter
    // produces from ordinary Markdown must have an HTML form.
    let corpus = "# H\n\npara **b** *i* ~~s~~ `c` [l](https://x.com)  \nnext\n\n\
                  1. one\n   - nested\n\n- [ ] task\n\n| a |\n| - |\n| b |\n\n\
                  ```sh\nx\n```\n\n> quote\n\n> [!NOTE]\n> panel\n\n---\n\n\
                  > # heading in quote\n\n- item\n\n  | t |\n  | - |\n  | v |\n";
    let out = html(corpus);
    assert!(out.starts_with("<h1>H</h1>"), "{out}");
}
