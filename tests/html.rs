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
    // Both are Confluence-only: Jira refuses layouts outright, and extensions
    // are macros with no Jira form.
    let embedded = doc(&json!([
        {"type": "paragraph", "content": [
            {"type": "inlineExtension", "attrs": {"extensionKey": "k", "extensionType": "t"}},
        ]},
        {"type": "layoutSection", "content": []},
    ]));
    let err = adf_to_html(&embedded).expect_err("unmapped nodes must be refused");
    let HtmlError::Unsupported(items) = &err else {
        panic!("expected Unsupported, got {err:?}");
    };
    assert_eq!(items.len(), 2, "{items:?}");
    assert!(items[0].contains("inlineExtension") && items[0].contains("/content/0/content/0"));
    assert!(items[1].contains("layoutSection") && items[1].contains("/content/1"));
}

#[test]
fn unmapped_marks_are_refused() {
    let marked = doc(&json!([{"type": "paragraph", "content": [
        {"type": "text", "text": "x", "marks": [{"type": "border", "attrs": {"size": 1, "color": "#000000"}}]},
    ]}]));
    let err = adf_to_html(&marked).expect_err("unmapped mark must be refused");
    assert!(err.to_string().contains("border"), "{err}");
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

// --- embed-reachable nodes and marks -----------------------------------------

/// Render one inline node inside a paragraph.
fn inline(node: &Value) -> String {
    let rendered = adf_to_html(&doc(&json!([{"type": "paragraph", "content": [node]}])))
        .unwrap_or_else(|e| panic!("render failed: {e}"));
    rendered
        .strip_prefix("<p>")
        .and_then(|s| s.strip_suffix("</p>"))
        .unwrap_or_else(|| panic!("not a single paragraph: {rendered}"))
        .to_string()
}

fn block(node: &Value) -> String {
    adf_to_html(&doc(&json!([node]))).unwrap_or_else(|e| panic!("render failed: {e}"))
}

#[test]
fn status_from_an_inline_embed() {
    // End to end from the Markdown an author writes.
    assert_eq!(
        html(
            "Build `adf:{\"type\":\"status\",\"attrs\":{\"text\":\"Done\",\"color\":\"green\"}}`."
        ),
        "<p>Build <span data-type=\"status\" data-color=\"green\">Done</span>.</p>"
    );
    assert_eq!(
        inline(&json!({"type": "status", "attrs": {"text": "x", "color": "red", "style": "bold"}})),
        "<span data-type=\"status\" data-color=\"red\" data-status-style=\"bold\">x</span>"
    );
}

#[test]
fn emoji_keeps_id_and_text_together() {
    assert_eq!(
        inline(
            &json!({"type": "emoji", "attrs": {"shortName": ":rocket:", "id": "1f680", "text": "\u{1f680}"}})
        ),
        "<span data-type=\"emoji\" data-shortname=\":rocket:\" data-emoji-id=\"1f680\" data-emoji-text=\"\u{1f680}\">\u{1f680}</span>"
    );
    // The guide wants both or neither, so a lone id is dropped and the server
    // fills both from the shortname.
    assert_eq!(
        inline(&json!({"type": "emoji", "attrs": {"shortName": ":smile:", "id": "1f604"}})),
        "<span data-type=\"emoji\" data-shortname=\":smile:\">:smile:</span>"
    );
}

#[test]
fn date_renders_its_calendar_day() {
    // 2025-03-15T00:00:00Z in milliseconds, ADF's timestamp unit.
    assert_eq!(
        inline(&json!({"type": "date", "attrs": {"timestamp": "1741996800000"}})),
        "<time datetime=\"2025-03-15\">2025-03-15</time>"
    );
    assert_eq!(
        inline(&json!({"type": "date", "attrs": {"timestamp": "0"}})),
        "<time datetime=\"1970-01-01\">1970-01-01</time>"
    );
}

#[test]
fn a_date_that_is_not_a_timestamp_is_refused() {
    let err = adf_to_html(&doc(&json!([{"type": "paragraph", "content": [
        {"type": "date", "attrs": {"timestamp": "yesterday"}},
    ]}])))
    .expect_err("not a timestamp");
    assert!(err.to_string().contains("timestamp"), "{err}");
}

#[test]
fn mention_and_placeholder() {
    assert_eq!(
        inline(
            &json!({"type": "mention", "attrs": {"id": "abc123", "text": "@Sam", "accessLevel": "CONTAINER", "userType": "DEFAULT"}})
        ),
        "<span data-type=\"mention\" data-user-id=\"abc123\" data-access-level=\"CONTAINER\" data-user-type=\"DEFAULT\">@Sam</span>"
    );
    assert_eq!(
        inline(&json!({"type": "placeholder", "attrs": {"text": "Type here"}})),
        "<span data-type=\"placeholder\">Type here</span>"
    );
}

#[test]
fn smart_cards() {
    assert_eq!(
        inline(&json!({"type": "inlineCard", "attrs": {"url": "https://x.com/a"}})),
        "<a href=\"https://x.com/a\" data-card-appearance=\"inline\"></a>"
    );
    // The canonical block-card form, which is also what the server echoes
    // back, so a read-modify-write of a fetched body stays byte-stable.
    assert_eq!(
        block(&json!({"type": "blockCard", "attrs": {"url": "https://x.com/b"}})),
        "<div data-type=\"block-card\" data-url=\"https://x.com/b\"><a href=\"https://x.com/b\">https://x.com/b</a></div>"
    );
    assert_eq!(
        block(
            &json!({"type": "embedCard", "attrs": {"url": "https://x.com/v", "layout": "wide", "width": 80}})
        ),
        "<div data-type=\"embed-card\" data-layout=\"wide\" data-width=\"80\"><iframe src=\"https://x.com/v\"></iframe></div>"
    );
}

#[test]
fn a_card_without_a_url_is_refused() {
    // The data and datasource variants carry resolved JSON-LD or a query, which
    // the dialect has no form for.
    let err = adf_to_html(&doc(&json!([{"type": "paragraph", "content": [
        {"type": "inlineCard", "attrs": {"data": {"name": "x"}}},
    ]}])))
    .expect_err("no url");
    assert!(err.to_string().contains("inlineCard"), "{err}");
}

#[test]
fn expand_and_nested_expand() {
    assert_eq!(
        block(
            &json!({"type": "expand", "attrs": {"title": "More"}, "content": [
                {"type": "paragraph", "content": [{"type": "text", "text": "hidden"}]},
            ]})
        ),
        "<details><summary>More</summary><p>hidden</p></details>"
    );
    assert_eq!(
        block(
            &json!({"type": "table", "content": [{"type": "tableRow", "content": [
                {"type": "tableCell", "content": [{"type": "nestedExpand", "attrs": {"title": "N"}, "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "x"}]},
                ]}]},
            ]}]})
        ),
        "<table><tr><td><details data-type=\"nested-expand\"><summary>N</summary><p>x</p></details></td></tr></table>"
    );
}

#[test]
fn decision_list() {
    assert_eq!(
        block(
            &json!({"type": "decisionList", "attrs": {"localId": "a"}, "content": [
                {"type": "decisionItem", "attrs": {"localId": "b", "state": "DECIDED"}, "content": [{"type": "text", "text": "Ship it"}]},
            ]})
        ),
        "<ul data-type=\"decision-list\"><li data-type=\"decision-item\" data-state=\"DECIDED\">Ship it</li></ul>"
    );
}

#[test]
fn block_task_item() {
    assert_eq!(
        block(
            &json!({"type": "taskList", "attrs": {"localId": "t1"}, "content": [
                {"type": "blockTaskItem", "attrs": {"localId": "t2", "state": "DONE"}, "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "x"}]},
                ]},
            ]})
        ),
        "<ul data-type=\"task-list\"><li data-type=\"block-task-item\"><input type=\"checkbox\" checked><p>x</p></li></ul>"
    );
}

#[test]
fn file_media_with_caption_and_group() {
    assert_eq!(
        block(
            &json!({"type": "mediaSingle", "attrs": {"layout": "center", "width": 80, "widthType": "percentage"}, "content": [
                {"type": "media", "attrs": {"type": "file", "id": "m1", "collection": "c1", "alt": "d.png"}},
                {"type": "caption", "content": [{"type": "text", "text": "Figure 1"}]},
            ]})
        ),
        "<figure data-type=\"media-single\" data-layout=\"center\" data-width=\"80\" data-width-type=\"percentage\">\
         <div data-type=\"media\" data-media-type=\"file\" data-id=\"m1\" data-collection=\"c1\" data-alt=\"d.png\"></div>\
         <figcaption>Figure 1</figcaption></figure>"
    );
    assert_eq!(
        block(&json!({"type": "mediaGroup", "content": [
            {"type": "media", "attrs": {"type": "file", "id": "m2", "collection": "c2"}},
        ]})),
        "<div data-type=\"media-group\"><div data-type=\"media\" data-media-type=\"file\" data-id=\"m2\" data-collection=\"c2\"></div></div>"
    );
    assert_eq!(
        inline(
            &json!({"type": "mediaInline", "attrs": {"type": "file", "id": "m3", "collection": "c3", "alt": "f.pdf"}})
        ),
        "<span data-type=\"media-inline\" data-media-type=\"file\" data-media-id=\"m3\" data-media-collection=\"c3\" data-alt=\"f.pdf\"></span>"
    );
}

#[test]
fn custom_panel() {
    assert_eq!(
        block(
            &json!({"type": "panel", "attrs": {"panelType": "custom", "panelIcon": ":star:", "panelColor": "#DEEBFF"}, "content": [
                {"type": "paragraph", "content": [{"type": "text", "text": "x"}]},
            ]})
        ),
        "<div data-type=\"panel-custom\" data-icon=\":star:\" data-color=\"#DEEBFF\"><p>x</p></div>"
    );
}

#[test]
fn tip_panel_is_refused_rather_than_rewritten() {
    // The server stores `panel-tip` as an info panel; emitting it would hand
    // back something other than what the document says, without a word.
    let err = adf_to_html(&doc(
        &json!([{"type": "panel", "attrs": {"panelType": "tip"}, "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": "x"}]},
        ]}]),
    ))
    .expect_err("tip has no lossless form");
    assert!(err.to_string().contains("tip"), "{err}");
}

#[test]
fn inline_style_marks() {
    let text = |marks: Value| inline(&json!({"type": "text", "text": "x", "marks": marks}));
    assert_eq!(text(json!([{"type": "underline"}])), "<u>x</u>");
    assert_eq!(
        text(json!([{"type": "subsup", "attrs": {"type": "sub"}}])),
        "<sub>x</sub>"
    );
    assert_eq!(
        text(json!([{"type": "subsup", "attrs": {"type": "sup"}}])),
        "<sup>x</sup>"
    );
    assert_eq!(
        text(json!([{"type": "textColor", "attrs": {"color": "#ff5630"}}])),
        "<span style=\"color: #ff5630\">x</span>"
    );
    assert_eq!(
        text(json!([{"type": "backgroundColor", "attrs": {"color": "#f8e6a0"}}])),
        "<span style=\"background-color: #f8e6a0\">x</span>"
    );
}

#[test]
fn annotation_wraps_everything_else() {
    // An inline comment anchors a whole run, link and styling included, so it
    // is the outermost wrapper.
    assert_eq!(
        inline(&json!({"type": "text", "text": "x", "marks": [
            {"type": "strong"},
            {"type": "link", "attrs": {"href": "https://x.com"}},
            {"type": "annotation", "attrs": {"id": "a1", "annotationType": "inlineComment"}},
        ]})),
        "<span class=\"annotation\" data-annotation-id=\"a1\" data-annotation-type=\"inlineComment\">\
         <a href=\"https://x.com\"><strong>x</strong></a></span>"
    );
    // It may also anchor a node, such as a status.
    assert_eq!(
        inline(
            &json!({"type": "status", "attrs": {"text": "x", "color": "blue"}, "marks": [
                {"type": "annotation", "attrs": {"id": "a2", "annotationType": "inlineComment"}},
            ]})
        ),
        "<span class=\"annotation\" data-annotation-id=\"a2\" data-annotation-type=\"inlineComment\">\
         <span data-type=\"status\" data-color=\"blue\">x</span></span>"
    );
}

#[test]
fn block_marks() {
    let para = |marks: Value| {
        block(
            &json!({"type": "paragraph", "marks": marks, "content": [{"type": "text", "text": "x"}]}),
        )
    };
    assert_eq!(
        para(json!([{"type": "alignment", "attrs": {"align": "center"}}])),
        "<p style=\"text-align: center\">x</p>"
    );
    assert_eq!(
        para(json!([{"type": "indentation", "attrs": {"level": 2}}])),
        "<p style=\"margin-left: 60px\">x</p>"
    );
    assert_eq!(
        para(json!([{"type": "fontSize", "attrs": {"fontSize": "small"}}])),
        "<p style=\"font-size: 14px\">x</p>"
    );
    assert_eq!(
        block(
            &json!({"type": "heading", "attrs": {"level": 2}, "marks": [{"type": "alignment", "attrs": {"align": "end"}}], "content": [{"type": "text", "text": "h"}]})
        ),
        "<h2 style=\"text-align: end\">h</h2>"
    );
    assert_eq!(
        block(
            &json!({"type": "codeBlock", "marks": [{"type": "breakout", "attrs": {"mode": "wide", "width": 1200}}], "content": [{"type": "text", "text": "x"}]})
        ),
        "<pre data-breakout=\"wide\" data-breakout-width=\"1200\"><code>x</code></pre>"
    );
}

#[test]
fn nesting_the_guide_forbids_in_a_panel_is_refused() {
    // The schema accepts a table in a panel; the server's guide excludes it
    // "for editor compatibility". Refused so the document is not stored in a
    // shape the editor cannot open.
    let err = adf_to_html(&doc(
        &json!([{"type": "panel", "attrs": {"panelType": "info"}, "content": [
            {"type": "table", "content": [{"type": "tableRow", "content": [
                {"type": "tableCell", "content": [{"type": "paragraph", "content": []}]},
            ]}]},
        ]}]),
    ))
    .expect_err("table in panel");
    let message = err.to_string();
    assert!(
        message.contains("table")
            && message.contains("panel")
            && message.contains("/content/0/content/0"),
        "{message}"
    );
}

#[test]
fn embedded_attribute_values_are_escaped() {
    assert_eq!(
        inline(&json!({"type": "status", "attrs": {"text": "<b>", "color": "red\" onclick=\"x"}})),
        "<span data-type=\"status\" data-color=\"red&quot; onclick=&quot;x\">&lt;b&gt;</span>"
    );
}
