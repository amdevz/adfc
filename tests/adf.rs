use adfc::markdown_to_adf;
use serde_json::{Value, json};

/// Validate a doc against the vendored ADF draft-04 schema.
///
/// Delegates to the library's own validator rather than compiling a second
/// one here: the shared validator is cached, so the whole suite pays the ~15ms
/// schema compile once instead of once per assertion, and these tests exercise
/// the same public API the CLI uses.
fn assert_valid_adf(doc: &Value) {
    if let Err(violations) = adfc::validate(doc) {
        panic!(
            "ADF schema violations:\n{violations}\ndoc: {}",
            serde_json::to_string_pretty(doc).unwrap()
        );
    }
}

fn convert(md: &str) -> Value {
    let doc = markdown_to_adf(md);
    assert_valid_adf(&doc);
    doc
}

#[test]
fn doc_envelope() {
    let doc = convert("hello");
    assert_eq!(doc["version"], 1);
    assert_eq!(doc["type"], "doc");
    assert_eq!(doc["content"][0]["type"], "paragraph");
    assert_eq!(doc["content"][0]["content"][0]["text"], "hello");
}

#[test]
fn headings_all_levels() {
    let doc = convert("# h1\n\n###### h6");
    assert_eq!(doc["content"][0]["type"], "heading");
    assert_eq!(doc["content"][0]["attrs"]["level"], 1);
    assert_eq!(doc["content"][1]["attrs"]["level"], 6);
}

#[test]
fn inline_marks() {
    let doc = convert("**b** *i* `c` ~~s~~ [t](https://x.com)");
    let inline = &doc["content"][0]["content"];
    let mark_of = |n: &Value| n["marks"][0]["type"].clone();
    assert_eq!(mark_of(&inline[0]), json!("strong"));
    assert_eq!(mark_of(&inline[2]), json!("em"));
    assert_eq!(mark_of(&inline[4]), json!("code"));
    assert_eq!(mark_of(&inline[6]), json!("strike"));
    let link = inline
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["marks"][0]["type"] == "link")
        .expect("link node");
    assert_eq!(link["marks"][0]["attrs"]["href"], "https://x.com");
}

#[test]
fn nested_marks() {
    let doc = convert("**bold *and italic***");
    let inline = doc["content"][0]["content"].as_array().unwrap();
    let both = inline
        .iter()
        .find(|n| n["marks"].as_array().is_some_and(|m| m.len() == 2))
        .expect("node with two marks");
    assert_eq!(both["text"], "and italic");
}

#[test]
fn bullet_list_wraps_items_in_paragraphs() {
    let doc = convert("- one\n- two");
    let list = &doc["content"][0];
    assert_eq!(list["type"], "bulletList");
    assert_eq!(list["content"][0]["type"], "listItem");
    // tight list items still need a block-level paragraph wrapper in ADF
    assert_eq!(list["content"][0]["content"][0]["type"], "paragraph");
    assert_eq!(
        list["content"][1]["content"][0]["content"][0]["text"],
        "two"
    );
}

#[test]
fn ordered_list_with_start() {
    let doc = convert("3. three\n4. four");
    let list = &doc["content"][0];
    assert_eq!(list["type"], "orderedList");
    assert_eq!(list["attrs"]["order"], 3);
}

#[test]
fn nested_lists() {
    let doc = convert("- a\n  - a1\n- b");
    let outer = &doc["content"][0];
    let first_item = &outer["content"][0];
    assert_eq!(first_item["content"][0]["type"], "paragraph");
    assert_eq!(first_item["content"][1]["type"], "bulletList");
}

#[test]
fn code_block_with_language() {
    let doc = convert("```rust\nfn main() {}\n```");
    let cb = &doc["content"][0];
    assert_eq!(cb["type"], "codeBlock");
    assert_eq!(cb["attrs"]["language"], "rust");
    assert_eq!(cb["content"][0]["text"], "fn main() {}");
}

#[test]
fn code_block_without_language() {
    let doc = convert("```\nplain\n```");
    let cb = &doc["content"][0];
    assert_eq!(cb["type"], "codeBlock");
    assert!(cb["attrs"].get("language").is_none() || cb["attrs"]["language"].is_null());
}

#[test]
fn blockquote() {
    let doc = convert("> quoted");
    assert_eq!(doc["content"][0]["type"], "blockquote");
    assert_eq!(doc["content"][0]["content"][0]["type"], "paragraph");
}

#[test]
fn table_with_header() {
    let doc = convert("| a | b |\n|---|---|\n| 1 | 2 |");
    let table = &doc["content"][0];
    assert_eq!(table["type"], "table");
    let head_row = &table["content"][0];
    assert_eq!(head_row["type"], "tableRow");
    assert_eq!(head_row["content"][0]["type"], "tableHeader");
    // header cell content must be block-level
    assert_eq!(head_row["content"][0]["content"][0]["type"], "paragraph");
    let body_row = &table["content"][1];
    assert_eq!(body_row["content"][0]["type"], "tableCell");
    assert_eq!(
        body_row["content"][1]["content"][0]["content"][0]["text"],
        "2"
    );
}

#[test]
fn rule_and_hard_break() {
    let doc = convert("a  \nb\n\n---");
    let para = &doc["content"][0]["content"];
    assert_eq!(para[1]["type"], "hardBreak");
    assert_eq!(doc["content"][1]["type"], "rule");
}

#[test]
fn soft_break_becomes_space() {
    let doc = convert("a\nb");
    let texts: Vec<String> = doc["content"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["text"].as_str().map(String::from))
        .collect();
    assert_eq!(texts.join(""), "a b");
}

#[test]
fn image_degrades_to_link() {
    let doc = convert("![alt text](https://x.com/i.png)");
    let inline = &doc["content"][0]["content"][0];
    assert_eq!(inline["text"], "alt text");
    assert_eq!(inline["marks"][0]["type"], "link");
    assert_eq!(inline["marks"][0]["attrs"]["href"], "https://x.com/i.png");
}

#[test]
fn image_alt_with_inline_code_keeps_literal_text() {
    let doc = convert("![see `config.rs`](https://x.com/i.png)");
    let inline = &doc["content"][0]["content"][0];
    assert_eq!(inline["text"], "see config.rs");
    assert_eq!(inline["marks"][0]["type"], "link");
}

#[test]
fn attachment_image_becomes_media_single() {
    let doc = convert("![](attachment:diagram.svg)");
    let media_single = &doc["content"][0];
    assert_eq!(media_single["type"], "mediaSingle");
    // `layout` is required by mediaSingle_node; the doc-level schema also
    // pins content to exactly one media child.
    assert_eq!(media_single["attrs"]["layout"], "center");
    assert_eq!(media_single["content"].as_array().unwrap().len(), 1);

    let media = &media_single["content"][0];
    assert_eq!(media["type"], "media");
    // `external`, not `file`: a file node additionally requires a media id and
    // collection, which the Jira REST API never exposes. The url stays a
    // placeholder that the apply step rewrites after uploading.
    assert_eq!(media["attrs"]["type"], "external");
    assert_eq!(media["attrs"]["url"], "attachment:diagram.svg");
}

#[test]
fn attachment_image_carries_alt_text() {
    let doc = convert("![Sequence diagram](attachment:diagram.svg)");
    let media = &doc["content"][0]["content"][0];
    assert_eq!(media["attrs"]["alt"], "Sequence diagram");
}

#[test]
fn attachment_image_without_alt_omits_the_attr() {
    let doc = convert("![](attachment:diagram.svg)");
    let media = &doc["content"][0]["content"][0];
    assert!(media["attrs"].get("alt").is_none());
}

#[test]
fn attachment_image_leaves_no_empty_paragraph() {
    let doc = convert("![](attachment:diagram.svg)");
    assert_eq!(doc["content"].as_array().unwrap().len(), 1);
}

#[test]
fn attachment_image_is_hoisted_out_of_a_mixed_paragraph() {
    // ADF paragraphs accept inline content only, so a media block sharing a
    // paragraph with text becomes a sibling rather than nesting.
    let doc = convert("before ![](attachment:diagram.svg) after");
    let types: Vec<&str> = doc["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert!(types.contains(&"mediaSingle"), "got {types:?}");
    assert!(types.contains(&"paragraph"), "got {types:?}");
}

#[test]
fn github_alert_blockquote_becomes_adf_panel() {
    for (marker, panel_type) in [
        ("NOTE", "note"),
        ("TIP", "success"),
        ("IMPORTANT", "info"),
        ("WARNING", "warning"),
        ("CAUTION", "error"),
    ] {
        let doc = convert(&format!("> [!{marker}]\n> body text"));
        let node = &doc["content"][0];
        assert_eq!(node["type"], "panel", "marker {marker}");
        assert_eq!(node["attrs"]["panelType"], panel_type, "marker {marker}");
        // The marker line is consumed, not rendered as content
        assert_eq!(node["content"][0]["type"], "paragraph");
        assert_eq!(node["content"][0]["content"][0]["text"], "body text");
    }
}

#[test]
fn plain_blockquote_is_still_a_blockquote() {
    let doc = convert("> just a quote");
    assert_eq!(doc["content"][0]["type"], "blockquote");
}

#[test]
fn task_list_becomes_adf_task_list() {
    let doc = convert("- [ ] todo item\n- [x] done item");
    let list = &doc["content"][0];
    assert_eq!(list["type"], "taskList");
    assert!(list["attrs"]["localId"].is_string());
    let first = &list["content"][0];
    assert_eq!(first["type"], "taskItem");
    assert_eq!(first["attrs"]["state"], "TODO");
    assert_eq!(first["content"][0]["text"], "todo item");
    assert_eq!(list["content"][1]["attrs"]["state"], "DONE");
}

#[test]
fn mixed_list_with_checkboxes_and_plain_items_stays_valid() {
    convert("- [ ] a\n- plain\n- [x] b");
}

#[test]
fn empty_input_yields_empty_doc() {
    let doc = convert("");
    assert_eq!(doc["content"].as_array().unwrap().len(), 0);
}

#[test]
fn kitchen_sink_validates() {
    convert(
        "# Title\n\nIntro **bold** and [link](https://a.b).\n\n\
         ## Section\n\n- item `code`\n- item two\n  1. nested\n\n\
         > note\n\n```sh\necho hi\n```\n\n| h |\n|---|\n| c |\n\n---\n\ndone",
    );
}

#[test]
fn inline_code_inside_bold_drops_the_incompatible_mark() {
    // ADF's code_inline_node permits only code, link and annotation alongside
    // code; formatted_text_inline_node permits everything except code. A text
    // node carrying both strong and code therefore matches neither, and the
    // API rejects the document.
    let doc = convert("**bold `c`**");
    let inline = &doc["content"][0]["content"];
    let coded = inline
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["text"] == "c")
        .expect("the code run survives");
    let marks: Vec<&str> = coded["marks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["type"].as_str().unwrap())
        .collect();
    assert_eq!(marks, vec!["code"], "code must not be combined with strong");
}

#[test]
fn inline_code_keeps_an_enclosing_link() {
    // link is one of the three marks ADF does allow next to code.
    let doc = convert("[see `c`](https://example.com)");
    let inline = &doc["content"][0]["content"];
    let coded = inline
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["text"] == "c")
        .expect("the code run survives");
    let marks: Vec<&str> = coded["marks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["type"].as_str().unwrap())
        .collect();
    assert!(marks.contains(&"code"), "got {marks:?}");
    assert!(marks.contains(&"link"), "got {marks:?}");
}

#[test]
fn inline_code_inside_em_and_strike_is_valid() {
    convert("*em `c`*");
    convert("~~struck `c`~~");
    convert("# Heading `c` with **bold `c`**");
}

#[test]
fn empty_table_cell_gets_an_empty_paragraph() {
    // ADF requires at least one block node in a cell (table_cell_content sets
    // minItems 1), but a content-less paragraph is dropped on the way out, so
    // an empty markdown cell would otherwise emit a cell with no content and
    // the API rejects the whole table.
    let doc = convert("| a | b |\n| - | - |\n| 1 |  |\n");
    let body_row = &doc["content"][0]["content"][1];
    let empty_cell = &body_row["content"][1];
    assert_eq!(empty_cell["type"], "tableCell");
    assert_eq!(
        empty_cell["content"][0]["type"], "paragraph",
        "empty cell must still hold a paragraph: {empty_cell}"
    );
}

#[test]
fn empty_table_header_gets_an_empty_paragraph() {
    let doc = convert("| a |  |\n| - | - |\n| 1 | 2 |\n");
    let header_row = &doc["content"][0]["content"][0];
    let empty_header = &header_row["content"][1];
    assert_eq!(empty_header["type"], "tableHeader");
    assert_eq!(empty_header["content"][0]["type"], "paragraph");
}

#[test]
fn a_table_of_entirely_empty_cells_is_still_valid() {
    convert("|  |  |\n| - | - |\n|  |  |\n");
}
