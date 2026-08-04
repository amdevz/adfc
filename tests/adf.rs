use adfc::markdown_to_adf;
use serde_json::{Value, json};

/// Validate a doc against the vendored ADF draft-04 schema.
///
/// Delegates to the library's own cached validator, so the suite pays the
/// ~15ms schema compile once and exercises the API the CLI uses.
fn assert_valid_adf(converted: &adfc::Conversion) {
    if let Err(violations) = adfc::validate(converted) {
        panic!(
            "ADF schema violations:\n{violations}\ndoc: {}",
            serde_json::to_string_pretty(converted.doc()).unwrap()
        );
    }
}

fn convert(md: &str) -> Value {
    let converted = markdown_to_adf(md);
    assert_valid_adf(&converted);
    converted.into_doc()
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

// --- ADF container restrictions ---------------------------------------------
//
// ADF is stricter than Markdown about what may nest inside a container. These
// cases are all valid Markdown, so they must degrade rather than produce a
// document the API rejects. `convert` validates, so reaching the assertions
// is itself the proof.

#[test]
fn heading_in_a_blockquote_becomes_a_bold_paragraph() {
    let doc = convert("> # Quoted heading\n");
    let quote = &doc["content"][0];
    assert_eq!(quote["type"], "blockquote");
    assert_eq!(quote["content"][0]["type"], "paragraph");
    assert_eq!(
        quote["content"][0]["content"][0]["marks"][0]["type"], "strong",
        "the heading's prominence is kept as emphasis: {quote}"
    );
    assert_eq!(quote["content"][0]["content"][0]["text"], "Quoted heading");
}

#[test]
fn heading_in_a_list_item_becomes_a_bold_paragraph() {
    let doc = convert("- item\n\n  # Nested heading\n");
    let item = &doc["content"][0]["content"][0];
    let types: Vec<&str> = item["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert!(!types.contains(&"heading"), "got {types:?}");
    assert!(types.contains(&"paragraph"));
}

#[test]
fn a_degraded_heading_leaves_an_inline_code_run_unbolded() {
    // ADF treats code as near-exclusive: beside it a text node may carry only
    // link and annotation. Bolding every run of a degraded heading therefore
    // produced a node matching neither code_inline_node nor
    // formatted_text_inline_node, and the whole document was refused.
    let doc = convert("> # a `c` b\n");
    let runs = doc["content"][0]["content"][0]["content"]
        .as_array()
        .expect("the degraded heading is a paragraph of runs");
    let code = runs
        .iter()
        .find(|run| run["text"] == "c")
        .expect("the code span survives: {doc}");
    let marks: Vec<&str> = code["marks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["type"].as_str().unwrap())
        .collect();
    assert_eq!(marks, ["code"], "code must not gain strong: {doc}");
    // The prominence is still carried by the runs that can hold it.
    assert_eq!(runs[0]["marks"][0]["type"], "strong", "got {doc}");
}

#[test]
fn a_degraded_heading_in_a_list_item_leaves_inline_code_unbolded() {
    let doc = convert("- # a `c` b\n");
    let runs = doc["content"][0]["content"][0]["content"][0]["content"]
        .as_array()
        .expect("the degraded heading is a paragraph of runs");
    let code = runs
        .iter()
        .find(|run| run["text"] == "c")
        .expect("the code span survives");
    assert_eq!(code["marks"].as_array().unwrap().len(), 1, "got {doc}");
}

#[test]
fn a_degraded_heading_does_not_mark_a_node_that_takes_no_marks() {
    // A status node carries no marks at all, so stamping one on it is invalid
    // in a way that has nothing to do with the code/strong conflict: only text
    // nodes can be emphasised.
    let doc = convert(
        "> # a `adf:{\"type\":\"status\",\"attrs\":{\"text\":\"D\",\"color\":\"green\"}}` b\n",
    );
    let runs = doc["content"][0]["content"][0]["content"]
        .as_array()
        .expect("the degraded heading is a paragraph of runs");
    let status = runs
        .iter()
        .find(|run| run["type"] == "status")
        .expect("the embedded badge survives");
    assert!(status["marks"].is_null(), "status takes no marks: {doc}");
}

#[test]
fn nested_blockquotes_flatten_into_one() {
    let doc = convert("> outer\n>\n> > inner\n");
    let quote = &doc["content"][0];
    assert_eq!(quote["type"], "blockquote");
    let types: Vec<&str> = quote["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert!(
        !types.contains(&"blockquote"),
        "ADF forbids nesting: {types:?}"
    );
    // Both texts survive the flattening.
    let rendered = quote.to_string();
    assert!(rendered.contains("outer") && rendered.contains("inner"));
}

#[test]
fn a_table_inside_a_list_item_is_hoisted_out() {
    let doc = convert("- item\n\n  | a | b |\n  | - | - |\n  | 1 | 2 |\n");
    let top: Vec<&str> = doc["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert!(top.contains(&"table"), "table should surface: {top:?}");
}

#[test]
fn a_table_inside_a_blockquote_is_hoisted_out() {
    let doc = convert("> | a | b |\n> | - | - |\n> | 1 | 2 |\n");
    let top: Vec<&str> = doc["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert!(top.contains(&"table"), "got {top:?}");
}

#[test]
fn a_panel_inside_a_list_item_unwraps() {
    let doc = convert("- item\n\n  > [!NOTE]\n  > careful\n");
    let item = &doc["content"][0]["content"][0];
    let types: Vec<&str> = item["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert!(!types.contains(&"panel"), "got {types:?}");
    assert!(item.to_string().contains("careful"), "text survives");
}

#[test]
fn a_rule_inside_a_list_item_is_dropped() {
    let doc = convert("- item\n\n  ---\n");
    let item = &doc["content"][0]["content"][0];
    let types: Vec<&str> = item["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert!(!types.contains(&"rule"), "got {types:?}");
}

#[test]
fn a_task_list_inside_a_blockquote_becomes_a_bullet_list() {
    // taskList is permitted in a list item but not in a blockquote.
    let doc = convert("> - [ ] quoted task\n");
    let quote = &doc["content"][0];
    let types: Vec<&str> = quote["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert!(!types.contains(&"taskList"), "got {types:?}");
    assert!(quote.to_string().contains("quoted task"));
}

#[test]
fn a_table_inside_a_panel_is_hoisted_out() {
    let doc = convert("> [!WARNING]\n> | a |\n> | - |\n> | 1 |\n");
    let top: Vec<&str> = doc["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert!(top.contains(&"table"), "got {top:?}");
}

#[test]
fn an_html_block_is_its_own_paragraph() {
    // Inline content is appended to a trailing paragraph when one is open, but
    // a block-level HTML run arriving after a closed paragraph was joining it,
    // silently merging two blocks into one line.
    let doc = convert("A paragraph here.\n\n<div>raw html</div>\n\nAnother paragraph.\n");
    let blocks = doc["content"].as_array().unwrap();
    assert_eq!(
        blocks.len(),
        3,
        "expected three blocks, got {}",
        blocks.len()
    );
    let first: String = blocks[0]["content"][0]["text"].as_str().unwrap().into();
    assert_eq!(
        first, "A paragraph here.",
        "the html leaked into the paragraph"
    );
}

#[test]
fn a_hoisted_table_follows_the_container_it_came_from() {
    // Hoisting happens while the enclosing container is still open, so pushing
    // straight to the ancestor put the table before the list it belonged to
    // and reversed the document's order.
    let doc = convert("- item mentioning the table\n\n  | a |\n  | - |\n  | 1 |\n");
    let types: Vec<&str> = doc["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        types,
        vec!["bulletList", "table"],
        "the table should follow the list, not precede it"
    );
}

#[test]
fn a_hoisted_table_follows_the_panel_it_came_from() {
    let doc = convert("> [!NOTE]\n> see below\n>\n> | a |\n> | - |\n> | 1 |\n");
    let types: Vec<&str> = doc["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, vec!["panel", "table"]);
}

#[test]
fn an_image_still_follows_the_text_that_introduced_it() {
    // Media hoists out of its paragraph; the words around it must keep their
    // position relative to it.
    let doc = convert("Intro text ![d](attachment:d.svg)\n");
    let types: Vec<&str> = doc["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, vec!["paragraph", "mediaSingle"]);
}

/// Markdown nesting `depth` levels of bullet list, one item per level.
///
/// Each Markdown level costs four JSON levels (list, item, paragraph, text), so
/// the emitted depth is roughly `4 * depth + 5`.
fn nested_list_markdown(depth: usize) -> String {
    (0..depth)
        .map(|i| format!("{}- item", "  ".repeat(i)))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn validation_refuses_a_document_past_the_depth_limit() {
    // The ADF schema is a recursive anyOf union, so branch exploration compounds
    // with nesting: this input is 41 KB and drove validation past 2 GB before it
    // was bounded, aborting the process. The refusal must come from the guard,
    // not from the validator running out of memory.
    let doc = markdown_to_adf(&nested_list_markdown(200));
    match adfc::validate(&doc) {
        Err(adfc::ValidationError::TooDeep { depth, limit }) => {
            assert_eq!(limit, adfc::MAX_VALIDATION_DEPTH);
            assert!(
                depth > limit,
                "reported depth {depth} should exceed the limit {limit}"
            );
        }
        other => panic!("expected TooDeep, got {other:?}"),
    }
}

#[test]
fn validation_accepts_ordinary_nesting() {
    // The guard must not turn real documents away. Twenty levels of list is
    // already deeper than hand-written content goes, and stays well inside the
    // limit.
    let doc = markdown_to_adf(&nested_list_markdown(20));
    assert!(adfc::validate(&doc).is_ok());
}

#[test]
fn the_depth_guard_reports_the_documents_own_depth() {
    // The error carries the actual depth so a caller can see how far over it is,
    // rather than only that some limit was hit.
    let doc = markdown_to_adf(&nested_list_markdown(50));
    let Err(adfc::ValidationError::TooDeep { depth, .. }) = adfc::validate(&doc) else {
        panic!("50 levels of list should exceed the limit");
    };
    // 4 levels of JSON per Markdown level, plus the doc/content wrapper.
    assert!(
        (200..=210).contains(&depth),
        "depth {depth} should track the document's real nesting"
    );
}

// --- the Conversion type ----------------------------------------------------

#[test]
fn conversion_exposes_the_document() {
    // markdown_to_adf now returns a Conversion rather than a bare Value, so the
    // document is reached through it.
    let c = markdown_to_adf("# Title");
    assert_eq!(c.doc()["type"], "doc");
    assert_eq!(c.doc()["content"][0]["type"], "heading");
}

#[test]
fn conversion_without_embeds_validates() {
    // Ordinary Markdown carries no embeds, so validation sees nothing new.
    let c = markdown_to_adf("Some **bold** text.");
    assert!(adfc::validate(&c).is_ok());
    assert!(c.embeds().is_empty(), "plain Markdown records no embeds");
}

#[test]
fn existing_markdown_converts_unchanged() {
    // The API changes shape; the conversion itself must not. A representative
    // document must still produce the same blocks in the same order.
    let c = markdown_to_adf(
        "# H\n\npara\n\n- a\n- b\n\n> quote\n\n| x |\n| - |\n| 1 |\n\n```rs\ncode\n```\n",
    );
    let types: Vec<&str> = c.doc()["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        types,
        vec![
            "heading",
            "paragraph",
            "bulletList",
            "blockquote",
            "table",
            "codeBlock"
        ]
    );
    assert!(adfc::validate(&c).is_ok());
}

// --- the adf fence ----------------------------------------------------------

/// A fenced block carrying `body` under the `adf` info string.
fn adf_fence(body: &str) -> String {
    format!("```adf\n{body}\n```\n")
}

#[test]
fn adf_fence_is_recognised() {
    let c = markdown_to_adf(&adf_fence(r#"{"type":"rule"}"#));
    assert_eq!(c.embeds().len(), 1, "the fence should record one embed");
}

#[test]
fn other_language_fence_is_still_a_code_block() {
    let c = markdown_to_adf("```rust\nfn main() {}\n```\n");
    assert!(c.embeds().is_empty());
    assert_eq!(c.doc()["content"][0]["type"], "codeBlock");
    assert_eq!(c.doc()["content"][0]["attrs"]["language"], "rust");
}

#[test]
fn fence_with_no_language_is_still_a_code_block() {
    let c = markdown_to_adf("```\nplain\n```\n");
    assert!(c.embeds().is_empty());
    assert_eq!(c.doc()["content"][0]["type"], "codeBlock");
}

#[test]
fn embedded_rule_appears_in_content() {
    let c = markdown_to_adf(&adf_fence(r#"{"type":"rule"}"#));
    assert_eq!(c.doc()["content"][0]["type"], "rule");
    assert!(adfc::validate(&c).is_ok());
}

#[test]
fn embedded_panel_is_a_real_panel() {
    let panel = r#"{"type":"panel","attrs":{"panelType":"info"},
        "content":[{"type":"paragraph","content":[{"type":"text","text":"hi"}]}]}"#;
    let c = markdown_to_adf(&adf_fence(panel));
    assert_eq!(c.doc()["content"][0]["type"], "panel");
    assert_eq!(c.doc()["content"][0]["attrs"]["panelType"], "info");
    assert!(adfc::validate(&c).is_ok());
}

#[test]
fn embed_keeps_document_order_relative_to_surrounding_prose() {
    let md = format!("before\n\n{}\nafter\n", adf_fence(r#"{"type":"rule"}"#));
    let c = markdown_to_adf(&md);
    let types: Vec<&str> = c.doc()["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, vec!["paragraph", "rule", "paragraph"]);
}

#[test]
fn an_embedded_paragraph_without_content_does_not_swallow_what_follows() {
    // An embed carries whatever the author wrote, including a paragraph with
    // no content array. Appending a later run into it would need an array that
    // is not there. The document is invalid ADF and validation says so; this is
    // only about nothing going missing.
    let md =
        "> ```adf\n> [{\"type\":\"paragraph\"},{\"type\":\"text\",\"text\":\"kept\"}]\n> ```\n";
    let converted = markdown_to_adf(md);
    let rendered = converted.doc().to_string();
    assert!(
        rendered.contains("kept"),
        "the run after the contentless paragraph is lost: {rendered}"
    );
}

#[test]
fn embed_values_pass_through_unchanged() {
    // attrs, content, marks and text must survive verbatim; the converter is a
    // conduit for an embed, not an editor of it.
    let node = r#"{"type":"paragraph","content":[
        {"type":"text","text":"kept","marks":[{"type":"strong"}]}]}"#;
    let c = markdown_to_adf(&adf_fence(node));
    let para = &c.doc()["content"][0];
    assert_eq!(para["content"][0]["text"], "kept");
    assert_eq!(para["content"][0]["marks"][0]["type"], "strong");
}

#[test]
fn malformed_embed_stays_visible_as_a_code_block() {
    let c = markdown_to_adf(&adf_fence(r#"{"type":"status",}"#));
    assert_eq!(
        c.doc()["content"][0]["type"],
        "codeBlock",
        "unparsed text must stay visible rather than vanish"
    );
    assert!(
        c.doc()["content"][0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("status"),
        "the author's text must be preserved verbatim"
    );
}

#[test]
fn malformed_embed_is_refused_by_validation() {
    let c = markdown_to_adf(&adf_fence(r#"{"type":"status",}"#));
    assert!(
        adfc::validate(&c).is_err(),
        "a codeBlock is valid ADF, so only the embed record can refuse this"
    );
    assert_eq!(c.embeds().len(), 1);
    assert!(c.embeds()[0].failure().is_some());
}

#[test]
fn malformed_embed_does_not_appear_as_an_adf_node() {
    let c = markdown_to_adf(&adf_fence(r#"{"type":"status",}"#));
    let types: Vec<&str> = c.doc()["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, vec!["codeBlock"], "no status node may be emitted");
}

#[test]
fn empty_fence_is_refused() {
    let c = markdown_to_adf("```adf\n```\n");
    assert!(
        adfc::validate(&c).is_err(),
        "an empty embed asks for nothing"
    );
}

#[test]
fn an_embed_a_container_forbids_is_refused_not_shipped() {
    // A rule cannot live in a listItem. What matters here is only that such
    // a document never silently passes; the targeted message and the choice
    // between refusing and relocating are pinned by their own cases below.
    let md = "- item\n\n  ```adf\n  {\"type\":\"rule\"}\n  ```\n";
    let c = markdown_to_adf(md);
    assert_eq!(c.embeds().len(), 1, "the fence is still recognised");
    assert!(
        adfc::validate(&c).is_err(),
        "an embed in a container that forbids it must not ship"
    );
}

// --- located, per-node embed errors -----------------------------------------

/// The rendered error for a source that should fail validation.
fn embed_error(md: &str) -> String {
    let c = markdown_to_adf(md);
    adfc::validate(&c)
        .expect_err("this source must fail validation")
        .to_string()
}

#[test]
fn embed_records_its_line() {
    let c = markdown_to_adf("intro\n\n```adf\n{\"type\":\"rule\"}\n```\n");
    assert_eq!(c.embeds().len(), 1);
    assert_eq!(c.embeds()[0].line(), 3, "the fence opens on line 3");
}

#[test]
fn second_embed_records_its_own_line() {
    let md = "```adf\n{\"type\":\"rule\"}\n```\n\ngap\n\n```adf\n{\"type\":\"rule\"}\n```\n";
    let c = markdown_to_adf(md);
    assert_eq!(c.embeds().len(), 2);
    assert_eq!(c.embeds()[0].line(), 1);
    assert_eq!(c.embeds()[1].line(), 7);
}

#[test]
fn misspelled_attribute_names_the_unexpected_key() {
    let e = embed_error(
        "```adf\n{\"type\":\"status\",\"attrs\":{\"text\":\"Done\",\"colour\":\"green\"}}\n```\n",
    );
    assert!(
        e.contains("'colour' was unexpected"),
        "should name the key the author actually wrote: {e}"
    );
}

#[test]
fn misspelled_attribute_names_the_expected_key() {
    let e = embed_error(
        "```adf\n{\"type\":\"status\",\"attrs\":{\"text\":\"Done\",\"colour\":\"green\"}}\n```\n",
    );
    assert!(
        e.contains("\"color\" is a required property"),
        "should name the key ADF actually wants: {e}"
    );
}

#[test]
fn value_outside_its_set_lists_the_allowed_values() {
    let e = embed_error(
        "```adf\n{\"type\":\"status\",\"attrs\":{\"text\":\"Done\",\"color\":\"orange\"}}\n```\n",
    );
    assert!(e.contains("\"orange\" is not one of"), "got: {e}");
    assert!(
        e.contains("neutral"),
        "should list the permitted values: {e}"
    );
}

#[test]
fn missing_required_attribute_is_named() {
    let e = embed_error("```adf\n{\"type\":\"status\",\"attrs\":{\"color\":\"green\"}}\n```\n");
    assert!(e.contains("\"text\" is a required property"), "got: {e}");
}

#[test]
fn unknown_node_type_is_named() {
    let e = embed_error("```adf\n{\"type\":\"statuz\"}\n```\n");
    assert!(
        e.contains("statuz"),
        "the unrecognised type must appear verbatim: {e}"
    );
}

#[test]
fn error_message_contains_the_line_number() {
    let e = embed_error("one\n\ntwo\n\n```adf\n{\"type\":\"statuz\"}\n```\n");
    assert!(e.contains("line 5"), "should locate the fence: {e}");
}

#[test]
fn unparsed_embed_error_carries_its_location() {
    let e = embed_error("intro\n\n```adf\n{\"type\":\"status\",}\n```\n");
    assert!(e.contains("line 3"), "got: {e}");
}

#[test]
fn no_embed_error_is_only_an_anyof_miss() {
    // The schema union message is unusable for an agent, so an embed failure
    // must never be reported with only that text.
    let e = embed_error(
        "```adf\n{\"type\":\"status\",\"attrs\":{\"text\":\"Done\",\"colour\":\"green\"}}\n```\n",
    );
    assert!(
        !e.contains("is not valid under any of the schemas"),
        "should be a targeted message, got: {e}"
    );
}

#[test]
fn a_valid_embedded_node_passes() {
    let c = markdown_to_adf("```adf\n{\"type\":\"rule\"}\n```\n");
    assert!(adfc::validate(&c).is_ok());
    assert!(c.embeds()[0].failure().is_none());
}

#[test]
fn embed_without_a_type_is_refused() {
    let e = embed_error("```adf\n{\"attrs\":{\"text\":\"x\"}}\n```\n");
    // Nearly every schema error contains the word "type", so assert the whole
    // phrase rather than a substring that cannot distinguish anything.
    assert!(
        e.contains("needs a \"type\" string"),
        "should name the missing key, got: {e}"
    );
}

#[test]
fn embed_that_is_not_an_object_is_refused() {
    let e = embed_error("```adf\n42\n```\n");
    assert!(e.contains("a number"), "should name what was found: {e}");
}

#[test]
fn synthesized_schema_declares_draft_04() {
    // The vendored schema is draft-04 and a synthesized per-node root does not
    // inherit that. Without the declaration every node fails to COMPILE rather
    // than to validate, and per-node checking silently stops happening.
    let e = embed_error(
        "```adf\n{\"type\":\"status\",\"attrs\":{\"text\":\"Done\",\"color\":\"orange\"}}\n```\n",
    );
    assert!(
        e.contains("not one of"),
        "a compiled per-node schema produces enum errors; got: {e}"
    );
}

// --- inline nodes, wrapped and in-sentence ----------------------------------

const STATUS: &str = r#"{"type":"status","attrs":{"text":"Done","color":"green"}}"#;

#[test]
fn inline_node_in_a_fence_is_wrapped_in_a_paragraph() {
    // A bare status cannot be a direct child of doc. Wrapping keeps the node
    // where the author put it and only adds the container ADF demands.
    let c = markdown_to_adf(&format!("```adf\n{STATUS}\n```\n"));
    assert_eq!(c.doc()["content"][0]["type"], "paragraph");
    assert_eq!(c.doc()["content"][0]["content"][0]["type"], "status");
    assert!(adfc::validate(&c).is_ok());
}

#[test]
fn wrapped_node_keeps_document_order() {
    let c = markdown_to_adf(&format!("before\n\n```adf\n{STATUS}\n```\n\nafter\n"));
    let types: Vec<&str> = c.doc()["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, vec!["paragraph", "paragraph", "paragraph"]);
    assert_eq!(c.doc()["content"][1]["content"][0]["type"], "status");
}

#[test]
fn block_node_in_a_fence_is_not_wrapped() {
    let c = markdown_to_adf("```adf\n{\"type\":\"rule\"}\n```\n");
    assert_eq!(c.doc()["content"][0]["type"], "rule");
}

#[test]
fn inline_adf_span_is_recognised() {
    let c = markdown_to_adf(&format!("The build is `adf:{STATUS}` and shipping.\n"));
    assert_eq!(c.embeds().len(), 1);
    assert!(adfc::validate(&c).is_ok());
}

#[test]
fn plain_code_span_is_untouched() {
    let c = markdown_to_adf("call `foo()` now\n");
    assert!(c.embeds().is_empty());
    let inline = &c.doc()["content"][0]["content"];
    assert_eq!(inline[1]["text"], "foo()");
    assert_eq!(inline[1]["marks"][0]["type"], "code");
}

#[test]
fn code_span_with_a_different_prefix_is_untouched() {
    let c = markdown_to_adf("see `adfx:{}` here\n");
    assert!(c.embeds().is_empty());
}

#[test]
fn text_badge_text_is_one_paragraph() {
    let c = markdown_to_adf(&format!("The build is `adf:{STATUS}` and shipping.\n"));
    let blocks = c.doc()["content"].as_array().unwrap();
    assert_eq!(blocks.len(), 1, "the badge must not break the paragraph");
    assert_eq!(blocks[0]["type"], "paragraph");
}

#[test]
fn badge_sits_between_its_text_runs() {
    let c = markdown_to_adf(&format!("The build is `adf:{STATUS}` and shipping.\n"));
    let inline = c.doc()["content"][0]["content"].as_array().unwrap();
    let types: Vec<&str> = inline.iter().map(|n| n["type"].as_str().unwrap()).collect();
    assert_eq!(types, vec!["text", "status", "text"]);
    assert_eq!(inline[0]["text"], "The build is ");
    assert_eq!(inline[2]["text"], " and shipping.");
}

#[test]
fn inline_embed_inside_a_list_item_stays_inline() {
    let c = markdown_to_adf(&format!("- item `adf:{STATUS}` here\n"));
    let item = &c.doc()["content"][0]["content"][0];
    assert_eq!(item["type"], "listItem");
    assert_eq!(item["content"][0]["type"], "paragraph");
    let types: Vec<&str> = item["content"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert!(types.contains(&"status"), "got {types:?}");
    assert!(adfc::validate(&c).is_ok());
}

#[test]
fn inline_span_carrying_a_block_node_is_refused() {
    let e = embed_error("text `adf:{\"type\":\"rule\"}` more\n");
    assert!(
        e.contains("rule") && e.contains("inline"),
        "should say a block node cannot sit inline: {e}"
    );
}

#[test]
fn a_malformed_inline_span_is_refused_with_its_line() {
    let e = embed_error("one\n\ntwo `adf:{\"type\":}` three\n");
    assert!(e.contains("line 3"), "got: {e}");
}

// --- schema-derived containment and arrays ----------------------------------

const TABLE: &str = r#"{"type":"table","content":[{"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph","content":[{"type":"text","text":"x"}]}]}]}]}"#;

#[test]
fn embedded_table_in_a_panel_is_refused() {
    let md = format!("> [!NOTE]\n> see below\n>\n> ```adf\n> {TABLE}\n> ```\n");
    let e = embed_error(&md);
    assert!(e.contains("table"), "should name the node: {e}");
    assert!(e.contains("panel"), "should name the container: {e}");
}

#[test]
fn an_embedded_node_is_never_relocated() {
    // Markdown-derived content hoists out of a container that rejects it,
    // because Markdown cannot express ADF's nesting rules. An embed names its
    // node explicitly, so moving it would change what was asked for.
    let md = format!("> [!NOTE]\n> see below\n>\n> ```adf\n> {TABLE}\n> ```\n");
    let c = markdown_to_adf(&md);
    let types: Vec<&str> = c.doc()["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert!(
        !types.contains(&"table"),
        "the embed must not be hoisted to the root: {types:?}"
    );
}

#[test]
fn an_embed_in_a_permitted_container_succeeds() {
    let md = "> [!NOTE]\n> see below\n>\n> ```adf\n> {\"type\":\"rule\"}\n> ```\n";
    let c = markdown_to_adf(md);
    assert!(adfc::validate(&c).is_ok(), "panel permits a rule");
}

#[test]
fn an_embedded_doc_is_refused() {
    let e = embed_error("```adf\n{\"type\":\"doc\",\"version\":1,\"content\":[]}\n```\n");
    // Not merely `contains("doc")`: the word appears in "document nests ..."
    // and in any union message, so that would pass against a broken guard.
    assert!(
        e.contains("doc is not allowed inside doc"),
        "a doc must be refused for its position, got: {e}"
    );
}

#[test]
fn markdown_content_still_hoists_out_of_a_panel() {
    // The regression that matters: replacing permits() must not change how
    // Markdown-authored content behaves.
    let doc = convert("> [!NOTE]\n> see below\n>\n> | a |\n> | - |\n> | 1 |\n");
    let types: Vec<&str> = doc["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, vec!["panel", "table"]);
}

#[test]
fn markdown_table_still_follows_the_list_it_came_from() {
    let doc = convert("- item mentioning the table\n\n  | a |\n  | - |\n  | 1 |\n");
    let types: Vec<&str> = doc["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, vec!["bulletList", "table"]);
}

#[test]
fn an_array_embed_produces_siblings_in_order() {
    let c = markdown_to_adf("```adf\n[{\"type\":\"rule\"},{\"type\":\"rule\"}]\n```\n");
    let types: Vec<&str> = c.doc()["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, vec!["rule", "rule"]);
    assert!(adfc::validate(&c).is_ok());
}

#[test]
fn an_array_element_that_is_not_an_object_is_refused() {
    let e = embed_error("```adf\n[{\"type\":\"rule\"},7]\n```\n");
    assert!(e.contains("a number"), "got: {e}");
}

#[test]
fn an_empty_array_embed_is_refused() {
    let e = embed_error("```adf\n[]\n```\n");
    assert!(e.contains("no nodes"), "got: {e}");
}

#[test]
fn several_nodes_in_an_inline_span_are_refused() {
    // The inline form takes exactly one node: a run of siblings has no single
    // position inside a sentence. A one-element array still delivers one node,
    // so it is accepted; two is the case with no honest reading.
    let e = embed_error(
        "text `adf:[{\"type\":\"emoji\",\"attrs\":{\"shortName\":\":a:\"}},{\"type\":\"emoji\",\"attrs\":{\"shortName\":\":b:\"}}]` more\n",
    );
    assert!(e.contains("one node"), "got: {e}");
}

// --- variants, marks and misplaced node types -------------------------------

#[test]
fn a_stricter_variant_does_not_fall_back_to_the_union_message() {
    // mediaSingle_node requires only `type`, but a document permits only the
    // stricter caption/full variants, which require `content`. Checking the
    // base let the node pass and the union report it, which is the one message
    // this feature exists to eliminate.
    let e = embed_error("```adf\n{\"type\":\"mediaSingle\"}\n```\n");
    assert!(
        !e.contains("is not valid under any of the schemas"),
        "got the union message: {e}"
    );
    assert!(e.contains("mediaSingle"), "should name the node: {e}");
    assert!(e.contains("content"), "should name what is missing: {e}");
}

#[test]
fn a_mark_embedded_as_a_node_is_named_as_not_a_node_type() {
    // `em` is a mark. No container accepts it, so reporting a placement
    // failure sends the author looking for a container that cannot exist.
    let e = embed_error("```adf\n{\"type\":\"em\"}\n```\n");
    assert!(
        e.contains("not an ADF node type"),
        "should say it is not a node at all: {e}"
    );
    assert!(
        !e.contains("not allowed inside"),
        "not a placement problem: {e}"
    );
}

#[test]
fn an_unknown_type_inline_is_named_as_not_a_node_type() {
    let e = embed_error("text `adf:{\"type\":\"statuz\"}` more\n");
    assert!(e.contains("not an ADF node type"), "got: {e}");
    assert!(
        !e.contains("is a block node"),
        "statuz is not a block node: {e}"
    );
}

#[test]
fn a_real_block_node_inline_is_still_named_as_a_block_node() {
    // The block-node message must survive for types that really are blocks.
    let e = embed_error("text `adf:{\"type\":\"rule\"}` more\n");
    assert!(e.contains("is a block node"), "got: {e}");
}

// --- depth bound and --schema parity for embeds -----------------------------

/// A bullet list nested `depth` levels around a paragraph.
///
/// Lists because ADF genuinely permits them to nest; a blockquote cannot hold
/// a blockquote, so that shape is refused before depth is reached. Each level
/// costs about four JSON levels.
fn nested_embed(depth: usize) -> String {
    let mut node = r#"{"type":"paragraph","content":[{"type":"text","text":"x"}]}"#.to_string();
    for _ in 0..depth {
        node = format!(
            r#"{{"type":"bulletList","content":[{{"type":"listItem","content":[{node}]}}]}}"#
        );
    }
    node
}

#[test]
fn a_deeply_nested_embed_never_reaches_the_validator() {
    // serde_json's parser stops at 128 levels, so an embed deeper than that is
    // refused before any schema work. The per-node depth guard is kept anyway
    // because that check also runs on nodes refused for their position, which
    // never reach the document guard.
    let c = markdown_to_adf(&format!("```adf\n{}\n```\n", nested_embed(40)));
    let err = adfc::validate(&c).expect_err("a node past the limit must be refused");
    let msg = err.to_string();
    assert!(
        msg.contains("recursion limit") || msg.contains("over the limit"),
        "must be refused by a bound rather than validated, got: {msg}"
    );
    // Whatever refused it, nothing may reach the document as an ADF node.
    assert_eq!(c.doc()["content"][0]["type"], "codeBlock");
}

#[test]
fn an_embed_within_the_depth_bound_still_validates() {
    let c = markdown_to_adf(&format!("```adf\n{}\n```\n", nested_embed(3)));
    assert!(
        adfc::validate(&c).is_ok(),
        "ordinary nesting must still pass"
    );
}

#[test]
fn validate_against_refuses_an_unhonoured_embed_too() {
    // The --schema path checked the document but never the embed record, so it
    // accepted a document whose embed was never honoured. A codeBlock is valid
    // ADF, so the document alone could never reveal that.
    let c = markdown_to_adf("```adf\n{\"type\":\"status\",}\n```\n");
    let schema: Value = serde_json::from_str(adfc::ADF_SCHEMA).expect("schema parses");
    assert!(
        adfc::validate_against(&schema, &c).is_err(),
        "--schema must apply the same embed guard as validate"
    );
}

#[test]
fn validate_against_still_accepts_a_good_document() {
    let c = markdown_to_adf("# Title\n\nSome text.\n");
    let schema: Value = serde_json::from_str(adfc::ADF_SCHEMA).expect("schema parses");
    assert!(adfc::validate_against(&schema, &c).is_ok());
}

#[test]
fn a_real_node_type_in_the_wrong_place_is_not_called_nonexistent() {
    // `tableRow` is defined by `table_row_node`, so guessing `<type>_node`
    // reported it as not being an ADF node type at all. The author's problem is
    // where the node sits, and a message about the name sends them to fix
    // something that is already right.
    let e = embed_error("```adf\n{\"type\":\"tableRow\",\"content\":[]}\n```\n");
    assert!(!e.contains("not an ADF node type"), "got: {e}");
    assert!(
        e.contains("tableRow is not allowed inside doc"),
        "should name the placement problem: {e}"
    );
}

#[test]
fn an_image_in_a_heading_becomes_a_sibling_of_the_heading() {
    // An ADF heading holds inline content only, so the media node cannot stay
    // inside it. Containment is what decides that, and `convert` checks the
    // result against the schema, so this pins the rule rather than the table it
    // came from.
    let doc = convert("# ![](attachment:d.svg)\n");
    let types: Vec<&str> = doc["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, vec!["heading", "mediaSingle"]);
}

#[test]
fn a_refusal_names_the_container_the_author_wrote() {
    // A checkbox list item is built as a listItem frame and only retagged to
    // taskItem when it closes, so an embed inside one used to be refused
    // against the pre-promotion name. The refusal is right either way, but it
    // named a container the author never wrote and the document never holds.
    let e = embed_error("- [ ] t\n\n  ```adf\n  {\"type\":\"rule\"}\n  ```\n");
    assert!(
        e.contains("inside taskItem"),
        "should name the task item, got: {e}"
    );
    assert!(
        !e.contains("inside listItem"),
        "must not name the intermediate frame: {e}"
    );
}

#[test]
fn a_plain_list_item_is_still_named_a_list_item() {
    // The promotion only applies to a checkbox item; an ordinary one must keep
    // its own name.
    let e = embed_error("- t\n\n  ```adf\n  {\"type\":\"rule\"}\n  ```\n");
    assert!(e.contains("inside listItem"), "got: {e}");
}
