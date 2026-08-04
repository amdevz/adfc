#![forbid(unsafe_code)]

//! Markdown -> Atlassian Document Format (ADF) conversion.
//!
//! The emitted document targets the official ADF JSON Schema
//! (<http://go.atlassian.com/adf-json-schema>, vendored in schema/).

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// The official Atlassian ADF JSON Schema, compiled into the binary.
///
/// Embedded rather than read from disk so an `npx adfc` or `cargo install adfc`
/// user, who has no checkout, can still validate. Refresh it from
/// <http://go.atlassian.com/adf-json-schema>; no code change is needed.
pub const ADF_SCHEMA: &str = include_str!("../schema/adf-schema.json");

/// [`ADF_SCHEMA`] parsed, once per process. Several indexes derive from it.
///
/// # Panics
///
/// Panics if the embedded schema is not valid JSON, for the reason given on
/// [`validator`].
fn schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        serde_json::from_str(ADF_SCHEMA).expect("vendored ADF schema is valid JSON")
    })
}

/// The `definitions` object of [`schema`], which every lookup here starts from.
fn definitions() -> &'static Value {
    &schema()["definitions"]
}

/// The compiled validator for [`ADF_SCHEMA`], built once per process.
///
/// Private: `jsonschema` is a 0.x dependency, so exposing its types would make
/// each of its breaking releases a breaking release of this crate. Compiling
/// costs ~15ms and dominates a conversion, hence the cache.
///
/// # Panics
///
/// Panics if the embedded schema fails to parse or compile — a build defect the
/// test suite catches, not a runtime condition a caller could handle.
fn validator() -> &'static jsonschema::Validator {
    static VALIDATOR: OnceLock<jsonschema::Validator> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| jsonschema::validator_for(schema()).expect("vendored ADF schema compiles"))
}

/// The compiled validator for one schema definition, built at most once.
///
/// [`check_embedded_node`] runs per embedded node and a document may carry
/// hundreds; compiling a root for each made 2000 inline embeds take 3.1s
/// against 0.04s without them. `None` if the definition does not exist or did
/// not compile, cached likewise.
fn definition_validator(definition: &str) -> Option<Arc<jsonschema::Validator>> {
    type Cache = Mutex<HashMap<String, Option<Arc<jsonschema::Validator>>>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();

    let cache = CACHE.get_or_init(Cache::default);
    // The guarded value is a pure memo, so a panic elsewhere cannot have left
    // it inconsistent; unwrapping would turn one unrelated panic into a panic
    // on every later validation.
    let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(cached) = cache.get(definition) {
        return cached.clone();
    }
    let compiled = node_schema_named(definition)
        .and_then(|root| jsonschema::validator_for(&root).ok())
        .map(Arc::new);
    cache.insert(definition.to_string(), compiled.clone());
    compiled
}

/// The deepest document [`validate`] and [`validate_against`] will check.
///
/// The ADF schema is a recursive `anyOf` union, so matching cost compounds with
/// nesting: 41 KB of nested lists (~800 levels) exhausted 2 GB and aborted.
/// Validation is on by default and [`markdown_to_adf`] cannot fail, so an
/// unbounded check hands that abort to anyone converting untrusted Markdown.
/// 128 is `serde_json`'s default recursion limit; real documents reach ~25.
pub const MAX_VALIDATION_DEPTH: usize = 128;

/// Why a document could not be validated.
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    /// Nesting exceeded [`MAX_VALIDATION_DEPTH`], so the document was refused
    /// without being checked. This says nothing about whether it is valid ADF,
    /// only that confirming it would cost more than the check is worth.
    #[error("document nests {depth} levels deep, over the limit of {limit}")]
    TooDeep {
        /// The document's actual nesting depth.
        depth: usize,
        /// The limit that was exceeded, always [`MAX_VALIDATION_DEPTH`].
        limit: usize,
    },
    /// An embed could not be honoured. Its text survives as a `codeBlock`,
    /// which is valid ADF, so nothing in the document itself records that
    /// something was asked for and not delivered. This does.
    #[error("{}", .0.join("\n"))]
    UnhonouredEmbeds(Vec<String>),
    /// The document was checked and violated the schema.
    #[error("{0}")]
    Violations(#[from] SchemaViolations),
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

/// A converted document, together with what the conversion found in embeds.
///
/// Paired so a caller cannot hold a document without its embed record: an embed
/// whose JSON does not parse degrades to a `codeBlock`, which is valid ADF, so
/// checking the document alone could never refuse it.
#[derive(Debug, Clone)]
pub struct Conversion {
    doc: Value,
    embeds: Vec<Embed>,
}

impl Conversion {
    /// The converted ADF document.
    #[must_use]
    pub fn doc(&self) -> &Value {
        &self.doc
    }

    /// Take the document, discarding the embed record.
    ///
    /// Named so that discarding the record is a visible decision.
    #[must_use]
    pub fn into_doc(self) -> Value {
        self.doc
    }

    /// Every raw ADF embed the source contained, in document order.
    #[must_use]
    pub fn embeds(&self) -> &[Embed] {
        &self.embeds
    }
}

/// A raw ADF node embedded in the Markdown source.
#[derive(Debug, Clone)]
pub struct Embed {
    line: usize,
    outcome: EmbedOutcome,
}

/// What became of an embed's body.
#[derive(Debug, Clone)]
enum EmbedOutcome {
    /// The nodes it carried, kept so validation can check each against its own
    /// schema definition rather than against the document-wide union.
    Nodes(Vec<Value>),
    /// The body could not be read as ADF nodes at all.
    Unparsed(String),
}

impl Embed {
    /// The 1-based source line the embed's fence opens on.
    ///
    /// A violation path like `/attrs/color` says what is wrong but not where.
    #[must_use]
    pub fn line(&self) -> usize {
        self.line
    }

    /// Why this embed could not become a node, when it could not.
    ///
    /// A failed embed is left in the document as visible text, so this is the
    /// only record of it. [`validate`] refuses a document carrying one.
    #[must_use]
    pub fn failure(&self) -> Option<&str> {
        match &self.outcome {
            EmbedOutcome::Unparsed(why) => Some(why),
            EmbedOutcome::Nodes(_) => None,
        }
    }
}

/// Validate a conversion against the embedded [`ADF_SCHEMA`].
///
/// ```
/// let converted = adfc::markdown_to_adf("# Title");
/// assert!(adfc::validate(&converted).is_ok());
/// assert_eq!(converted.doc()["content"][0]["type"], "heading");
/// ```
///
/// # Errors
///
/// [`ValidationError::TooDeep`] if the document nests beyond
/// [`MAX_VALIDATION_DEPTH`], checked before any validation work. Otherwise
/// every violation found, not just the first.
pub fn validate(converted: &Conversion) -> Result<(), ValidationError> {
    // Depth first, before anything validates a node. Checking embeds first
    // would hand a pathologically nested embed straight to the schema, which
    // is the unbounded cost [`MAX_VALIDATION_DEPTH`] exists to prevent.
    guard_depth(&converted.doc)?;
    guard_embeds(converted)?;
    validate_document(&converted.doc)
}

/// Refuse a conversion whose embeds cannot be honoured, before the document is
/// checked at all: a body that did not parse leaves a valid `codeBlock`, so the
/// document alone would pass, and a node violating its own definition would be
/// reported as a bare `anyOf` miss naming neither the node nor the field.
fn guard_embeds(converted: &Conversion) -> Result<(), ValidationError> {
    let mut failures = Vec::new();
    for embed in &converted.embeds {
        match &embed.outcome {
            EmbedOutcome::Unparsed(why) => {
                failures.push(format!("line {}: {why}", embed.line));
            }
            EmbedOutcome::Nodes(nodes) => {
                for node in nodes {
                    failures.extend(
                        check_embedded_node(node)
                            .into_iter()
                            .map(|v| format!("line {}: {v}", embed.line)),
                    );
                }
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(ValidationError::UnhonouredEmbeds(failures))
    }
}

/// Check one embedded node against the schema definition for its own type.
///
/// Targeting the definition rather than the document union turns an `anyOf`
/// miss into `Additional properties are not allowed ('colour' was unexpected)`.
fn check_embedded_node(node: &Value) -> Vec<String> {
    // Bounded here too, not only at the document: an embed refused for its
    // position never reaches the document, so guarding only there would leave
    // this path unbounded.
    let depth = nesting_depth(node);
    if depth > MAX_VALIDATION_DEPTH {
        return vec![format!(
            "nests {depth} levels deep, over the limit of {MAX_VALIDATION_DEPTH}"
        )];
    }
    let Some(node_type) = node["type"].as_str() else {
        return vec!["an embedded node needs a \"type\" string".into()];
    };
    // A type's own definition can be laxer than the variants a container
    // accepts: mediaSingle_node requires only `type` where the caption and full
    // variants require `content`. Passing any one variant is enough.
    let mut closest: Option<Vec<String>> = None;
    for variant in variants_of(node_type) {
        let Some(validator) = definition_validator(variant) else {
            continue;
        };
        let errors: Vec<String> = validator
            .iter_errors(node)
            .map(|e| embed_violation(node_type, &e))
            .collect();
        if errors.is_empty() {
            return Vec::new();
        }
        // Report the closest near-miss rather than every variant's complaints,
        // which would bury the likely intent.
        if closest
            .as_ref()
            .is_none_or(|best| errors.len() < best.len())
        {
            closest = Some(errors);
        }
    }
    if let Some(errors) = closest {
        return errors;
    }

    // No variants, or none that compiled. Falling through rather than
    // reporting the node clean: it has not been checked yet.
    let Some(definition) = node_definition(node_type) else {
        return vec![format!("\"{node_type}\" is not an ADF node type")];
    };
    let Some(validator) = definition_validator(definition) else {
        // The synthesized root failed to compile. That is a defect in this
        // function rather than in the author's node, so say nothing and let
        // document validation report what it can.
        return Vec::new();
    };
    validator
        .iter_errors(node)
        .map(|e| embed_violation(node_type, &e))
        .collect()
}

/// One violation of an embedded node, located when there is a location.
///
/// A failure of the node itself carries an empty instance path, so appending it
/// unconditionally leaves the message trailing `at ` and pointing nowhere.
fn embed_violation(node_type: &str, error: &jsonschema::ValidationError<'_>) -> String {
    let path = error.instance_path.to_string();
    if path.is_empty() {
        format!("{node_type}: {error}")
    } else {
        format!("{node_type}: {error} at {path}")
    }
}

/// The definition name of every stricter variant of a node type, if it has any.
///
/// A variant resolves to the same node type but adds constraints, such as
/// `mediaSingle_full_node` requiring `content` where `mediaSingle_node` does
/// not. Empty for a type with none, so the caller falls back to the base. Names
/// rather than schemas, so each can be looked up in [`definition_validator`].
fn variants_of(node_type: &str) -> &'static [String] {
    static VARIANTS: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();
    let index = VARIANTS.get_or_init(|| {
        let definitions = definitions();
        let mut index: HashMap<String, Vec<String>> = HashMap::new();
        let Some(entries) = definitions.as_object() else {
            return index;
        };
        for name in entries.keys() {
            // Only a variant, never the base: the base is the fallback and
            // including it would let a lax definition pass on its own.
            if !name.ends_with("_node") || definitions[name]["allOf"].as_array().is_none() {
                continue;
            }
            if let Some(resolved) = type_of_definition(definitions, name, &mut Vec::new()) {
                index.entry(resolved).or_default().push(name.clone());
            }
        }
        index
    });
    index.get(node_type).map_or(&[], Vec::as_slice)
}

/// The definition describing a node type.
///
/// Usually `<type>_node`, but not always: `tableRow`, `tableCell` and
/// `tableHeader` come from `table_row_node` and friends, so guessing the name
/// reported three real ADF types as nonexistent. Derived from the schema.
fn node_definition(node_type: &str) -> Option<&'static str> {
    static INDEX: OnceLock<HashMap<String, String>> = OnceLock::new();
    let index = INDEX.get_or_init(|| {
        let mut index: HashMap<String, String> = HashMap::new();
        let Some(entries) = definitions().as_object() else {
            return index;
        };
        for (name, definition) in entries {
            // A mark states its type the same way -- `em_mark` is `"em"` -- and
            // must keep answering that it is not a node. The suffix is what
            // separates the two.
            if !name.ends_with("_node") {
                continue;
            }
            let Some(declared) = definition["properties"]["type"]["enum"]
                .as_array()
                .and_then(|enumerated| enumerated.first())
                .and_then(Value::as_str)
            else {
                continue;
            };
            // The conventional name wins wherever it exists, so `codeBlock`
            // keeps `codeBlock_node` rather than `codeBlock_root_only_node`.
            let conventional = name
                .strip_suffix("_node")
                .is_some_and(|stem| stem == declared);
            let slot = index
                .entry(declared.to_string())
                .or_insert_with(|| name.clone());
            if conventional {
                slot.clone_from(name);
            }
        }
        index
    });
    index.get(node_type).map(String::as_str)
}

/// A schema rooted at one node definition, carrying the vendored definitions so
/// its internal references still resolve.
///
/// The `$schema` declaration is not optional: the vendored schema is draft-04
/// and a synthesized root does not inherit it, so without it every node fails
/// to compile rather than to validate.
fn node_schema_named(definition: &str) -> Option<Value> {
    let definitions = definitions();
    definitions.get(definition)?;
    Some(json!({
        "$schema": "http://json-schema.org/draft-04/schema#",
        "$ref": format!("#/definitions/{definition}"),
        "definitions": definitions.clone(),
    }))
}

/// Validate an ADF document that did not come from [`markdown_to_adf`].
///
/// Reach for [`validate`] after a conversion instead: it also accounts for the
/// embeds the conversion found.
///
/// # Errors
///
/// The same conditions as [`validate`], minus anything embed-related.
pub fn validate_document(doc: &Value) -> Result<(), ValidationError> {
    guard_depth(doc)?;
    Ok(validate_with(validator(), doc)?)
}

/// Validate a document against an arbitrary ADF schema.
///
/// Backs the CLI's `--schema` override. Takes a [`Value`] rather than a
/// compiled validator so no `jsonschema` type appears in the public API.
///
/// # Errors
///
/// [`SchemaError::InvalidSchema`] if `schema` is not usable as a JSON Schema.
/// Otherwise the same conditions as [`validate`]; the [`MAX_VALIDATION_DEPTH`]
/// bound applies here too.
pub fn validate_against(schema: &Value, converted: &Conversion) -> Result<(), SchemaError> {
    let validator =
        jsonschema::validator_for(schema).map_err(|e| SchemaError::InvalidSchema(e.to_string()))?;
    guard_depth(&converted.doc)?;
    // The same embed guard [`validate`] applies. Omitting it here made the
    // `--schema` flag a way to accept a document whose embeds were never
    // honoured, which is the hole [`Conversion`] exists to close.
    guard_embeds(converted)?;
    Ok(validate_with(&validator, &converted.doc).map_err(ValidationError::Violations)?)
}

/// The failure modes of validating against a caller-supplied schema.
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    /// The supplied schema could not be compiled.
    #[error("not a usable JSON Schema: {0}")]
    InvalidSchema(String),
    /// The document could not be validated against it.
    #[error("{0}")]
    Validation(#[from] ValidationError),
}

/// Refuse a document whose nesting would make validation disproportionately
/// expensive, before any validator touches it.
fn guard_depth(doc: &Value) -> Result<(), ValidationError> {
    let depth = nesting_depth(doc);
    if depth > MAX_VALIDATION_DEPTH {
        return Err(ValidationError::TooDeep {
            depth,
            limit: MAX_VALIDATION_DEPTH,
        });
    }
    Ok(())
}

/// Depth of the most deeply nested container, counting the root as 1.
///
/// Iterative by necessity: a recursive walk would overflow the stack on exactly
/// the documents this guard exists to reject.
fn nesting_depth(doc: &Value) -> usize {
    let mut deepest = 0;
    let mut stack = vec![(doc, 1usize)];
    while let Some((node, depth)) = stack.pop() {
        deepest = deepest.max(depth);
        match node {
            Value::Object(map) => stack.extend(map.values().map(|v| (v, depth + 1))),
            Value::Array(items) => stack.extend(items.iter().map(|v| (v, depth + 1))),
            _ => {}
        }
    }
    deepest
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
/// `![alt](attachment:diagram.svg)` becomes a media node whose url stays the
/// placeholder, for an uploader to rewrite. Keeps the conversion a pure
/// function with no network access.
pub const ATTACHMENT_SCHEME: &str = "attachment:";

/// Convert a Markdown string into an ADF document (`{version: 1, type: "doc", ...}`).
///
/// Never fails: unrepresentable constructs degrade rather than error (remote
/// images become labeled links, raw HTML is kept as plain text). Images using
/// the [`ATTACHMENT_SCHEME`] become real media nodes instead.
///
/// Returns a [`Conversion`] so an embed that could not be honoured travels with
/// the document and [`validate`] can refuse it.
///
/// ```
/// let converted = adfc::markdown_to_adf("# Title");
/// let doc = converted.doc();
/// assert_eq!(doc["content"][0]["type"], "heading");
/// assert_eq!(doc["content"][0]["attrs"]["level"], 1);
/// ```
#[must_use]
pub fn markdown_to_adf(markdown: &str) -> Conversion {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(markdown, options).into_offset_iter();
    let mut builder = Builder::new();
    for (event, range) in parser {
        builder.event(event, range.start);
    }
    // Offsets become line numbers here, where the source is still in hand;
    // carrying it into the builder would tie the builder to a lifetime it does
    // not need.
    let embeds = std::mem::take(&mut builder.embeds)
        .into_iter()
        .map(|(offset, outcome)| Embed {
            line: line_of(markdown, offset),
            outcome,
        })
        .collect();
    Conversion {
        doc: builder.finish(),
        embeds,
    }
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
    /// Nodes lifted out of a container that cannot hold them, paired with the
    /// stack depth they belong at. Emitted once that container closes, so a
    /// hoisted table follows the list it came from instead of preceding it.
    hoisted: Vec<(usize, Value)>,
    /// Raw ADF embeds in document order, each with the byte offset it was
    /// written at. Travels out with the document so validation can refuse one
    /// that could not be honoured; `markdown_to_adf` resolves the offsets.
    embeds: Vec<(usize, EmbedOutcome)>,
    /// Whether the open code block is an `adf` fence. Fences cannot nest, so a
    /// single flag is enough.
    in_adf_fence: bool,
    /// Byte offset the open fence started at, resolved to a line once the walk
    /// finishes.
    fence_offset: usize,
}

/// Which node types each container may hold directly, taken from the schema.
///
/// Restating it in code would duplicate several hundred rules and drift the
/// moment Atlassian revises them. A hand-written table also only ever covered
/// the ~20 types this converter emits, not the 43 an embed can reach.
fn containment() -> &'static HashMap<String, Vec<String>> {
    static TABLE: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let definitions = definitions();
        let mut table = HashMap::new();
        let Some(entries) = definitions.as_object() else {
            return table;
        };
        for name in entries.keys() {
            let Some(parent) = type_of_definition(definitions, name, &mut Vec::new()) else {
                continue;
            };
            // `content` is sometimes a $ref to a shared definition rather than
            // an inline array, as for a table cell. Missing that indirection
            // drops the container's rules and would let a table into a cell.
            let content = &definitions[name]["properties"]["content"];
            let content = content["$ref"]
                .as_str()
                .and_then(|r| r.strip_prefix("#/definitions/"))
                .map_or(content, |target| &definitions[target]);
            let items = &content["items"];
            let refs = match items["anyOf"].as_array() {
                Some(any) => any.clone(),
                None if items["$ref"].is_string() => vec![items.clone()],
                None => continue,
            };
            let mut children: Vec<String> = Vec::new();
            let mut seen: Vec<String> = Vec::new();
            for reference in &refs {
                collect_child_types(definitions, reference, &mut children, &mut seen);
            }
            // Variants of one type each contribute what they allow; a node is
            // permitted if any variant of its parent accepts it.
            let slot: &mut Vec<String> = table.entry(parent).or_default();
            for child in children {
                if !slot.contains(&child) {
                    slot.push(child);
                }
            }
        }
        table
    })
}

/// Every node type one content reference admits, unions included.
///
/// A reference names either a node definition or a union of them: a paragraph
/// states its content as `items.$ref` to `inline_node`, an `anyOf` with no type
/// of its own. Resolving only the direct name leaves such a container with an
/// empty entry, which forbids everything it actually holds.
fn collect_child_types(
    definitions: &Value,
    reference: &Value,
    out: &mut Vec<String>,
    seen: &mut Vec<String>,
) {
    let Some(base) = reference["$ref"]
        .as_str()
        .and_then(|r| r.strip_prefix("#/definitions/"))
    else {
        return;
    };
    // Unions may name each other, so a definition is expanded once.
    if seen.iter().any(|s| s == base) {
        return;
    }
    seen.push(base.to_string());
    if let Some(node_type) = type_of_definition(definitions, base, &mut Vec::new()) {
        out.push(node_type);
        return;
    }
    if let Some(branches) = definitions[base]["anyOf"].as_array() {
        for branch in branches {
            collect_child_types(definitions, branch, out, seen);
        }
    }
}

/// Whether ADF permits `child` directly inside `parent`.
fn permits(parent: &str, child: &str) -> bool {
    match containment().get(parent) {
        Some(children) => children.iter().any(|c| c == child),
        // A container the schema does not describe. Staying permissive keeps
        // the hoisting walk working: it looks outwards for somewhere a node is
        // legal, and a false here would stop it at the first unknown frame.
        None => true,
    }
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

/// Info string marking a fenced block as a raw ADF embed rather than source text.
const ADF_FENCE: &str = "adf";

/// Prefix marking an inline code span as a raw ADF embed.
const INLINE_ADF_PREFIX: &str = "adf:";

/// Resolve a schema definition name to the ADF node type it describes.
///
/// Variant definitions carry no `type` enum and extend a base through `allOf`,
/// so `paragraph_with_no_marks_node` must be followed to `paragraph_node`. The
/// `seen` set guards against a cycle.
fn type_of_definition(definitions: &Value, name: &str, seen: &mut Vec<String>) -> Option<String> {
    if seen.iter().any(|s| s == name) {
        return None;
    }
    seen.push(name.to_string());
    let definition = definitions.get(name)?;
    if let Some(enumerated) = definition["properties"]["type"]["enum"].as_array()
        && let Some(node_type) = enumerated.first().and_then(Value::as_str)
    {
        return Some(node_type.to_string());
    }
    let variants = definition["allOf"].as_array()?;
    variants.iter().find_map(|variant| {
        let reference = variant["$ref"].as_str()?;
        let base = reference.strip_prefix("#/definitions/")?;
        type_of_definition(definitions, base, seen)
    })
}

/// Every node type ADF treats as inline, from the schema's own `inline_node`
/// union. A hardcoded list would be wrong the moment Atlassian adds a type, and
/// would silently place a new inline node as a block.
fn inline_types() -> &'static [String] {
    static INLINE: OnceLock<Vec<String>> = OnceLock::new();
    INLINE.get_or_init(|| {
        let definitions = definitions();
        let mut types: Vec<String> = definitions["inline_node"]["anyOf"]
            .as_array()
            .map(|refs| {
                refs.iter()
                    .filter_map(|r| {
                        let base = r["$ref"].as_str()?.strip_prefix("#/definitions/")?;
                        type_of_definition(definitions, base, &mut Vec::new())
                    })
                    .collect()
            })
            .unwrap_or_default();
        types.sort();
        types.dedup();
        types
    })
}

/// Whether ADF places this node type inside a paragraph rather than beside one.
fn is_inline(node_type: &str) -> bool {
    inline_types().iter().any(|t| t == node_type)
}

/// The 1-based line containing `offset`.
fn line_of(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1
}

/// Parse an embed body into the nodes it carries.
///
/// Accepts a single node object or an array of them. Returns the reason as a
/// string rather than a typed error: it is destined for a human-readable
/// refusal, and `serde_json`'s parse position comes free.
fn parse_embed(body: &str) -> Result<Vec<Value>, String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err("the embed is empty".into());
    }
    let parsed: Value = serde_json::from_str(trimmed).map_err(|e| format!("invalid JSON: {e}"))?;
    let nodes = match parsed {
        Value::Array(items) => items,
        node @ Value::Object(_) => vec![node],
        other => {
            return Err(format!(
                "expected an ADF node object or an array of them, found {}",
                kind_of(&other)
            ));
        }
    };
    if nodes.is_empty() {
        return Err("the embed carries no nodes".into());
    }
    for node in &nodes {
        if !node.is_object() {
            return Err(format!(
                "every embedded node must be an object, found {}",
                kind_of(node)
            ));
        }
        if !node["type"].is_string() {
            return Err("every embedded node needs a \"type\" string".into());
        }
    }
    Ok(nodes)
}

/// Whether the schema describes this as a node, as opposed to a mark or
/// something invented. Checked before placement: a mark, or a typo, has no
/// container anywhere, so reporting it as a nesting problem misleads.
fn is_node_type(node_type: &str) -> bool {
    node_definition(node_type).is_some()
}

/// Refuse any node the container forbids, naming both.
fn reject_forbidden(container: &str, nodes: &[Value]) -> Result<(), String> {
    for node in nodes {
        let child = node["type"].as_str().unwrap_or_default();
        if !is_node_type(child) {
            return Err(format!("\"{child}\" is not an ADF node type"));
        }
        // An inline node is wrapped in a paragraph before it is appended, so
        // the container sees that paragraph rather than the node itself.
        let effective = if is_inline(child) { "paragraph" } else { child };
        if !permits(container, effective) {
            return Err(format!(
                "{child} is not allowed inside {container}; ADF forbids that nesting"
            ));
        }
    }
    Ok(())
}

/// Name a JSON value's kind for an error message.
fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Read a `[!MARKER]` alert tag from the start of a blockquote's first text run.
fn alert_marker(text: &str) -> Option<&'static str> {
    let rest = text.trim_start().strip_prefix("[!")?;
    let marker = rest.split(']').next()?;
    panel_type_for(marker)
}

/// Carry a degraded heading's prominence into one of its runs.
///
/// Only a text node can hold the mark: `status`, `emoji` and the rest accept no
/// marks at all, and ADF forbids `strong` beside `code`. Marking them anyway
/// produced a node matching no inline variant, refusing the whole document —
/// from Markdown as ordinary as `> # a ``c`` b`.
fn emphasise(run: &mut Value) {
    if run["type"] != "text" {
        return;
    }
    let Some(marks) = run
        .as_object_mut()
        .map(|object| object.entry("marks").or_insert_with(|| json!([])))
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    if marks
        .iter()
        .any(|mark| mark["type"] == "code" || mark["type"] == "strong")
    {
        return;
    }
    marks.push(json!({"type": "strong"}));
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
            hoisted: Vec::new(),
            embeds: Vec::new(),
            in_adf_fence: false,
            fence_offset: 0,
        }
    }

    /// Take an inline `adf:` span: splice its node into the surrounding text,
    /// or leave the span as visible code and record why it could not be used.
    ///
    /// Only inline nodes are accepted. Placing a block node here would break
    /// the paragraph the author was writing, so it is refused, not relocated.
    fn inline_embed(&mut self, body: &str, offset: usize) {
        match parse_embed(body).and_then(|nodes| {
            // A run of siblings has no single position inside a sentence. A
            // one-element array still delivers one node, so it is accepted.
            if nodes.len() > 1 {
                return Err("an inline embed carries exactly one node".to_string());
            }
            Ok(nodes)
        }) {
            Ok(nodes)
                if nodes
                    .iter()
                    .all(|n| n["type"].as_str().is_some_and(is_inline)) =>
            {
                self.embeds
                    .push((offset, EmbedOutcome::Nodes(nodes.clone())));
                for node in nodes {
                    self.append_block_or_inline(node);
                }
            }
            Ok(nodes) => {
                let offender = nodes
                    .iter()
                    .find(|n| !n["type"].as_str().is_some_and(is_inline))
                    .and_then(|n| n["type"].as_str())
                    .unwrap_or("that node")
                    .to_string();
                // An unrecognised type is not a block node; it is not a node.
                // Calling it a block would send the author to a fenced block
                // that will fail in exactly the same way.
                let why = if is_node_type(&offender) {
                    format!(
                        "{offender} is a block node and cannot sit inline; use a fenced adf block"
                    )
                } else {
                    format!("\"{offender}\" is not an ADF node type")
                };
                self.embeds.push((offset, EmbedOutcome::Unparsed(why)));
                self.keep_span_as_code(body);
            }
            Err(failure) => {
                self.embeds.push((offset, EmbedOutcome::Unparsed(failure)));
                self.keep_span_as_code(body);
            }
        }
    }

    /// Emit any inline nodes gathered from a fence as one paragraph of their
    /// own, preserving the order they were written in.
    fn flush_pending_inline(&mut self, pending: &mut Vec<Value>) {
        if pending.is_empty() {
            return;
        }
        let content = Value::Array(std::mem::take(pending));
        self.append_block(json!({"type": "paragraph", "content": content}));
    }

    /// Leave an unusable inline embed visible, as the code span it was written
    /// as. Losing what the author wrote is worse than emitting something
    /// validation will refuse anyway.
    fn keep_span_as_code(&mut self, body: &str) {
        let node = json!({
            "type": "text",
            "text": format!("{INLINE_ADF_PREFIX}{body}"),
            "marks": [{"type": "code"}],
        });
        self.append_block_or_inline(node);
    }

    /// Close an `adf` fence: splice its nodes in, or keep the text visible and
    /// record why it could not be honoured.
    ///
    /// On a successful parse the codeBlock frame holding the author's text is
    /// discarded and replaced by the nodes; on failure it is kept, because
    /// losing what the author wrote is worse than emitting something invalid.
    fn finish_adf_fence(&mut self, body: &str) {
        match parse_embed(body).and_then(|nodes| {
            // The frame below the fence is the real container. An embed names
            // its node explicitly, so one the container forbids is refused
            // rather than hoisted: relocating it would change what was asked.
            let container = self
                .stack
                .get(self.stack.len().saturating_sub(2))
                .map_or("doc", |f| f.node_type);
            // A checkbox item is a listItem frame until it closes, so
            // reporting that name would name a container the author never wrote
            // and the document will never hold.
            let container = if container == "listItem" && self.task_state.is_some() {
                "taskItem"
            } else {
                container
            };
            reject_forbidden(container, &nodes).map(|()| nodes)
        }) {
            Ok(nodes) => {
                // Discard the frame rather than pop() it: its text was the
                // source of the nodes, not content of its own.
                self.stack.pop();
                self.embeds
                    .push((self.fence_offset, EmbedOutcome::Nodes(nodes.clone())));
                // A fence is its own block, so an inline node in one gets a
                // fresh paragraph; append_block_or_inline would let it join
                // whatever paragraph closed last.
                let mut pending: Vec<Value> = Vec::new();
                for node in nodes {
                    if node["type"].as_str().is_some_and(is_inline) {
                        pending.push(node);
                    } else {
                        self.flush_pending_inline(&mut pending);
                        self.append_block_or_inline(node);
                    }
                }
                self.flush_pending_inline(&mut pending);
                // pop() guarantees this on every exit path, and bypassing it
                // above would strand anything queued for this depth.
                self.flush_hoisted();
            }
            Err(failure) => {
                self.embeds
                    .push((self.fence_offset, EmbedOutcome::Unparsed(failure)));
                self.pop();
            }
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
        self.pop_frame();
        // Unconditional: pop_frame returns early for promoted and emptied
        // nodes, and anything queued for this depth must still be emitted.
        self.flush_hoisted();
    }

    fn pop_frame(&mut self) {
        let mut frame = self.stack.pop().expect("balanced events");

        // A list item carrying a checkbox becomes a taskItem, whose content is
        // INLINE (no paragraph wrapper) per the ADF schema.
        if frame.node_type == "listItem"
            && let Some(state) = self.task_state.take()
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
            self.append_block(json!({
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

        // ADF requires at least one block node in a table cell, but an empty
        // markdown cell produces no events, so the cell closed empty and the
        // API rejected the whole table. Atlassian's editor stores this too.
        if matches!(frame.node_type, "tableCell" | "tableHeader") && frame.content.is_empty() {
            frame
                .content
                .push(json!({"type": "paragraph", "content": []}));
        }

        // These require at least one child and can be emptied by hoisting
        // their only content out. Emitting the husk would fail validation and
        // it carries nothing, so drop it.
        if matches!(frame.node_type, "blockquote" | "panel" | "listItem")
            && frame.content.is_empty()
        {
            return;
        }
        let mut node = Map::new();
        node.insert("type".into(), json!(frame.node_type));
        if let Some(attrs) = frame.attrs {
            node.insert("attrs".into(), attrs);
        }
        node.insert("content".into(), Value::Array(frame.content));
        self.append_block(Value::Object(node));
    }

    /// Append a block-level node, hoisting it past any enclosing paragraph.
    ///
    /// ADF paragraphs accept inline content only, so a block emitted while a
    /// paragraph frame is open (an image, which the parser always reports inside
    /// one) becomes the paragraph's sibling instead of its child. `pop` drops a
    /// paragraph left empty, so no stray one trails the block.
    fn append_hoisted_block(&mut self, node: Value) {
        let child = node["type"].as_str().unwrap_or_default();
        // Walk out to the nearest ancestor that accepts this node. Stopping at
        // the first non-paragraph is not enough: a table inside a list item
        // would land straight back in the list item, which forbids it.
        let idx = self
            .stack
            .iter()
            .rposition(|frame| frame.node_type != "paragraph" && permits(frame.node_type, child))
            .unwrap_or(0);

        if idx == self.stack.len() - 1 {
            self.stack[idx].content.push(node);
        } else {
            // The target is still open further up, so emitting now would place
            // this ahead of everything between here and there. Queue it until
            // that frame is the innermost one again.
            self.hoisted.push((idx, node));
        }
    }

    /// Emit anything queued for the frame that is now innermost.
    fn flush_hoisted(&mut self) {
        let depth = self.stack.len();
        if depth == 0 {
            return;
        }
        let mut ready = Vec::new();
        self.hoisted.retain(|(idx, node)| {
            if *idx == depth - 1 {
                ready.push(node.clone());
                false
            } else {
                true
            }
        });
        for node in ready {
            self.stack[depth - 1].content.push(node);
        }
    }

    /// Append a finished block node, degrading it if ADF forbids it here.
    ///
    /// A heading inside a blockquote, a nested blockquote, a table inside a
    /// list item are all ordinary Markdown and all illegal ADF. Emitting them
    /// produces a document the API rejects wholesale, so each is reshaped into
    /// something the container accepts and the content is kept.
    fn append_block(&mut self, node: Value) {
        let parent = self
            .stack
            .last()
            .map_or("doc", |frame| frame.node_type)
            .to_owned();
        let parent = if parent == "paragraph" {
            // A block closing while a paragraph is open belongs to whatever
            // encloses the paragraph.
            self.stack
                .iter()
                .rev()
                .find(|f| f.node_type != "paragraph")
                .map_or("doc", |f| f.node_type)
                .to_owned()
        } else {
            parent
        };

        let child = node["type"].as_str().unwrap_or_default().to_owned();
        if permits(&parent, &child) {
            self.append_block_or_inline(node);
            return;
        }

        match child.as_str() {
            // Carries no content of its own, so there is nothing to preserve.
            "rule" => {}
            // Keep the words and their prominence; ADF has no heading here.
            "heading" => {
                let mut para = node;
                para["type"] = json!("paragraph");
                para.as_object_mut().map(|o| o.remove("attrs"));
                if let Some(runs) = para["content"].as_array_mut() {
                    for run in runs.iter_mut() {
                        emphasise(run);
                    }
                }
                self.append_block_or_inline(para);
            }
            // Unwrap: the children are usually paragraphs the container allows,
            // and each is re-checked on the way in.
            "blockquote" | "panel" => {
                if let Some(children) = node["content"].as_array() {
                    for child in children.clone() {
                        self.append_block(child);
                    }
                }
            }
            // A blockquote may hold a bullet list but not a task list.
            "taskList" => {
                let items: Vec<Value> = node["content"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .map(|item| {
                                json!({
                                    "type": "listItem",
                                    "content": [{
                                        "type": "paragraph",
                                        "content": item["content"].clone(),
                                    }],
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.append_block_or_inline(json!({"type": "bulletList", "content": items}));
            }
            // Everything else, tables included, is lifted to the nearest
            // ancestor that accepts it rather than having its content dropped.
            _ => self.append_hoisted_block(node),
        }
    }

    /// Append a finished node to the current frame, wrapping inline nodes in a
    /// paragraph when the container requires block-level children.
    fn append_block_or_inline(&mut self, node: Value) {
        let is_inline = node["type"].as_str().is_some_and(is_inline);
        let frame = self.stack.last_mut().expect("non-empty stack");
        if is_inline && frame.wraps_inline {
            // Append into a trailing paragraph, creating one if needed. The
            // test is for a content array, not merely a paragraph: an embed can
            // leave `{"type": "paragraph"}` behind, which would drop the node.
            let reusable = matches!(
                frame.content.last(),
                Some(last) if last["type"] == "paragraph" && last["content"].is_array()
            );
            if !reusable {
                frame
                    .content
                    .push(json!({"type": "paragraph", "content": []}));
            }
            // Infallible by construction: the trailing element is either the
            // paragraph just pushed or one the check above confirmed.
            if let Some(runs) = frame
                .content
                .last_mut()
                .and_then(|para| para["content"].as_array_mut())
            {
                runs.push(node);
            }
        } else {
            frame.content.push(node);
        }
    }

    /// Promote `> [!NOTE]`-style blockquotes to ADF panels.
    ///
    /// Runs as the first paragraph of a blockquote closes: the parser splits
    /// `[!NOTE]` across several text events, so the marker is only detectable
    /// once the runs are joined.
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

    fn event(&mut self, event: Event, offset: usize) {
        match event {
            Event::Start(tag) => self.start(tag, offset),
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
                // A code span prefixed `adf:` carries a raw node, so a badge
                // can sit inside a sentence. Fences are always block-level and
                // cannot do that.
                if let Some(body) = t.strip_prefix(INLINE_ADF_PREFIX) {
                    self.inline_embed(body, offset);
                    return;
                }
                // ADF treats code as near-exclusive: alongside it a text node
                // may carry only link and annotation, so emitting the enclosing
                // strong/em/strike produces a node the API rejects. The emphasis
                // goes rather than the code, which carries the meaning.
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
            Event::Rule => self.append_block(json!({"type": "rule"})),
            // Raw HTML has no ADF equivalent; keep it visible as plain text.
            Event::Html(t) | Event::InlineHtml(t) => self.text(t.trim_end_matches('\n')),
            Event::FootnoteReference(_) => {}
        }
    }

    fn start(&mut self, tag: Tag, offset: usize) {
        match tag {
            // A block of raw HTML gets a paragraph of its own. Without a
            // frame it is treated as loose inline content and appended to
            // whatever paragraph closed last, merging two blocks into one.
            Tag::Paragraph | Tag::HtmlBlock => self.push("paragraph", None, false),
            Tag::Heading { level, .. } => {
                let level = heading_level(level);
                self.push("heading", Some(json!({ "level": level })), false);
            }
            Tag::BlockQuote(_) => self.push("blockquote", None, true),
            Tag::CodeBlock(kind) => {
                // An `adf` fence carries a raw node, not source text. It still
                // opens a codeBlock frame: the text accumulates the same way,
                // and the frame is the fallback when the JSON does not parse.
                self.in_adf_fence = matches!(
                    &kind,
                    CodeBlockKind::Fenced(lang) if lang.as_ref() == ADF_FENCE
                );
                if self.in_adf_fence {
                    self.fence_offset = offset;
                }
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
            | TagEnd::HtmlBlock
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
                if std::mem::take(&mut self.in_adf_fence) {
                    self.finish_adf_fence(&merged);
                } else {
                    self.pop();
                }
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
                    // `external`, not `file`: a file node also requires a media
                    // id and collection, which Jira's REST API never exposes for
                    // an attachment. The url stays the placeholder for the apply
                    // step to rewrite after upload.
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
    fn derived_containment_permits_everything_the_hardcoded_table_did() {
        // Every rule the old hand-written table STATED must survive. Not that
        // nothing changed: it ended in a permissive default, so containers it
        // never named allowed everything and are now narrower. The one
        // reachable case is pinned by the image-in-a-heading test.
        let previous: &[(&str, &[&str])] = &[
            (
                "listItem",
                &[
                    "paragraph",
                    "bulletList",
                    "orderedList",
                    "codeBlock",
                    "taskList",
                    "mediaSingle",
                ],
            ),
            (
                "blockquote",
                &[
                    "paragraph",
                    "bulletList",
                    "orderedList",
                    "codeBlock",
                    "mediaSingle",
                ],
            ),
            (
                "panel",
                &[
                    "paragraph",
                    "bulletList",
                    "orderedList",
                    "codeBlock",
                    "taskList",
                    "mediaSingle",
                    "heading",
                    "rule",
                ],
            ),
            ("bulletList", &["listItem"]),
            ("orderedList", &["listItem"]),
            ("taskList", &["taskItem"]),
            ("table", &["tableRow"]),
            ("tableRow", &["tableCell", "tableHeader"]),
        ];
        for (parent, children) in previous {
            for child in *children {
                assert!(
                    permits(parent, child),
                    "derived containment lost a rule: {parent} used to permit {child}"
                );
            }
        }
    }

    #[test]
    fn derived_containment_still_forbids_a_table_anywhere() {
        // No container this converter emits may hold a table, which is what
        // makes hoisting necessary at all.
        for parent in [
            "listItem",
            "blockquote",
            "panel",
            "tableCell",
            "tableHeader",
        ] {
            assert!(
                !permits(parent, "table"),
                "{parent} must not permit a table"
            );
        }
    }

    #[test]
    fn containment_expands_a_union_content_reference() {
        // A heading and a paragraph state their content as `items.$ref` to
        // `inline_node`, a union with no type of its own. Resolving only the
        // direct name left both entries empty, which forbids everything.
        assert!(permits("heading", "status"), "a heading holds inline nodes");
        assert!(
            permits("paragraph", "text"),
            "a paragraph holds inline nodes"
        );
        // The rules hoisting depends on must survive that expansion: neither
        // may hold a block node, or an image in a heading would stay there and
        // the document would fail the schema.
        assert!(!permits("heading", "mediaSingle"));
        assert!(!permits("paragraph", "table"));
    }

    #[test]
    fn every_container_the_converter_emits_carries_derived_rules() {
        // A container the derivation cannot read falls back to permissive; one
        // read as empty forbids everything. Both are silent, so the shapes are
        // pinned here rather than discovered as mangled output.
        for parent in [
            "doc",
            "paragraph",
            "heading",
            "blockquote",
            "panel",
            "listItem",
            "bulletList",
            "orderedList",
            "taskList",
            "table",
            "tableRow",
            "tableCell",
            "tableHeader",
            "codeBlock",
        ] {
            let children = containment().get(parent);
            assert!(
                children.is_some_and(|children| !children.is_empty()),
                "{parent} has no derived rules: {children:?}"
            );
        }
    }

    #[test]
    fn the_table_node_types_are_recognised_as_node_types() {
        // Their definitions are `table_row_node` and friends, so guessing
        // `<type>_node` called three real ADF types nonexistent.
        for node_type in ["tableRow", "tableCell", "tableHeader"] {
            assert!(is_node_type(node_type), "{node_type} is an ADF node type");
        }
        // A mark states its type the same way and must still be refused, and
        // so must a name that is simply wrong.
        assert!(!is_node_type("em"), "em is a mark, not a node");
        assert!(!is_node_type("statuz"), "statuz is nothing at all");
    }

    #[test]
    fn containment_is_derived_once() {
        assert!(std::ptr::eq(containment(), containment()));
    }

    #[test]
    fn embedded_schema_parses() {
        let schema: Value =
            serde_json::from_str(ADF_SCHEMA).expect("embedded schema is valid JSON");
        assert_eq!(schema["$schema"], "http://json-schema.org/draft-04/schema#");
    }

    #[test]
    fn the_schema_is_parsed_once() {
        assert!(std::ptr::eq(schema(), schema()));
    }

    #[test]
    fn a_definition_validator_is_compiled_once() {
        // Compiling a root per embedded node made 2000 inline embeds take
        // 3.1s against 0.04s for the same document without them.
        let first = definition_validator("status_node").expect("status_node is a definition");
        let second = definition_validator("status_node").expect("status_node is a definition");
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn a_definition_that_does_not_exist_stays_a_miss() {
        // The negative result is cached too, so a repeated miss must keep
        // answering None rather than being recompiled into something.
        assert!(definition_validator("statuz_node").is_none());
        assert!(definition_validator("statuz_node").is_none());
    }

    #[test]
    fn every_variant_of_a_type_compiles() {
        // A variant that fails to compile is skipped silently, which would
        // quietly reduce embed checking to the base definition — the laxer
        // check this function exists to replace.
        let variants = variants_of("mediaSingle");
        assert!(!variants.is_empty(), "mediaSingle has stricter variants");
        for variant in variants {
            assert!(
                definition_validator(variant).is_some(),
                "{variant} must compile"
            );
        }
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
        let converted = markdown_to_adf("# Title\n\nSome **bold** text.");
        assert!(validate(&converted).is_ok());
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
        assert!(validate_document(&doc).is_err());
    }

    #[test]
    fn violations_render_one_per_line() {
        let doc = json!({"version": 1, "type": "doc", "content": [
            {"type": "heading", "attrs": {"level": 99}, "content": [{"type": "text", "text": "a"}]},
            {"type": "heading", "attrs": {"level": 42}, "content": [{"type": "text", "text": "b"}]},
        ]});
        let err = validate_document(&doc).expect_err("invalid doc must fail validation");
        let rendered = err.to_string();
        assert!(rendered.lines().count() >= 2, "got: {rendered}");
    }

    #[test]
    fn violations_carry_instance_paths() {
        let doc = json!({"version": 1, "type": "doc", "content": [
            {"type": "heading", "attrs": {"level": 99}, "content": [{"type": "text", "text": "a"}]},
        ]});
        let err = validate_document(&doc).expect_err("invalid doc must fail validation");
        assert!(err.to_string().contains("/content/0"), "got: {err}");
    }

    #[test]
    fn empty_document_is_valid() {
        let converted = markdown_to_adf("");
        assert_eq!(converted.doc()["content"], json!([]));
        assert!(validate(&converted).is_ok());
    }
}
