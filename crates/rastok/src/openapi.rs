// This module is build tooling: discovering and updating project files is its
// explicit effect boundary, not an observation made by the semantic core.
#![allow(clippy::disallowed_methods)]
//! Project-wide OpenAPI discovery and Rastok type generation.
//!
//! The resolver is build tooling, not runtime protocol machinery. It parses
//! `.rkg` files through the real host parser, finds statically named HTTP
//! endpoints, matches them to OpenAPI 3 documents below the project root, and
//! writes one deterministic generated Rastok source file. User-authored files
//! are never rewritten in place.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::compiler::syntax::{
    ConnectorDecl, GraphDecl, GraphItem, OperatorItem, SyntaxTree, TopDecl, Value as SynValue,
};
use crate::datatype::{
    BinaryType, DataType, DateUnit, FloatType, IntegerType, IntegerWidth, TemporalType, TimeUnit,
    TimeZone,
};
use crate::diagnostics::{Bag, SourceId};
use crate::expr::{self, Signature};
use crate::value::Value;

/// File maintained by [`resolve_project`].
pub const GENERATED_FILE: &str = "openapi.generated.rkg";

/// Whether project resolution may update the generated source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolveMode {
    /// Write the deterministic output when it differs.
    Write,
    /// Refuse when the generated source is absent or stale.
    Check,
}

/// Observable result of one project resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolveReport {
    /// Generated source path.
    pub output: PathBuf,
    /// Number of static HTTP endpoints matched.
    pub endpoints: usize,
    /// Number of OpenAPI documents selected by those endpoints.
    pub specifications: usize,
    /// Number of generated Rastok aliases.
    pub types: usize,
    /// Whether write mode changed the generated file.
    pub changed: bool,
}

/// Typed project-resolution refusal.
#[derive(Debug)]
pub enum ResolveError {
    /// Project root or one discovered file could not be read or written.
    Io {
        /// Path whose operation failed.
        path: PathBuf,
        /// Operating-system refusal.
        source: std::io::Error,
    },
    /// A Rastok source containing an HTTP endpoint does not parse cleanly.
    Source {
        /// Source or project path.
        path: PathBuf,
        /// Stable human explanation.
        detail: String,
    },
    /// An HTTP endpoint is not statically discoverable.
    DynamicEndpoint {
        /// Declaring Rastok source.
        path: PathBuf,
        /// Connector instance name.
        connector: String,
        /// Endpoint attribute that was not constant.
        field: &'static str,
    },
    /// No HTTP endpoint was found under the project root.
    NoEndpoints,
    /// No OpenAPI document describes one discovered endpoint.
    MissingEndpoint {
        /// Client method, or `None` for an inbound server path.
        method: Option<String>,
        /// Exact request path.
        path: String,
    },
    /// More than one OpenAPI document claims one endpoint.
    AmbiguousEndpoint {
        /// Client method, or `None` for an inbound server path.
        method: Option<String>,
        /// Exact request path.
        path: String,
        /// Conflicting documents in deterministic path order.
        documents: Vec<PathBuf>,
    },
    /// One selected OpenAPI schema cannot project without loss.
    Projection {
        /// Selected OpenAPI document.
        document: PathBuf,
        /// Located lossless-projection refusal.
        source: crate::schema_projection::ProjectionError,
    },
    /// Two selected specifications define the same name differently.
    ConflictingType {
        /// Shared component name.
        name: String,
        /// First selected document.
        first: PathBuf,
        /// Differing selected document.
        second: PathBuf,
    },
    /// A generated component would duplicate a user-owned type declaration.
    AuthoredTypeConflict {
        /// Shared type name.
        name: String,
        /// Authored Rastok source that owns it.
        source: PathBuf,
    },
    /// An external name cannot be written as one Rastok identifier.
    UnrepresentableName {
        /// Document declaring the name.
        document: PathBuf,
        /// External type, field, case, or enum spelling.
        name: String,
    },
    /// A projected logical type has no source spelling in the current grammar.
    UnrepresentableType {
        /// Document declaring the type.
        document: PathBuf,
        /// Stable description of the unsupported canonical type.
        ty: String,
    },
    /// Check mode found a missing or stale generated source.
    Stale {
        /// Generated source that needs regeneration.
        path: PathBuf,
    },
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "cannot access `{}`: {source}", path.display()),
            Self::Source { path, detail } => {
                write!(
                    f,
                    "cannot inspect endpoint source `{}`: {detail}",
                    path.display()
                )
            }
            Self::DynamicEndpoint {
                path,
                connector,
                field,
            } => write!(
                f,
                "`{connector}` in `{}` has a dynamic `{field}`; OpenAPI discovery requires a constant endpoint",
                path.display()
            ),
            Self::NoEndpoints => {
                f.write_str("the project declares no HttpClient or HttpServer endpoint")
            }
            Self::MissingEndpoint { method, path } => match method {
                Some(method) => write!(f, "no OpenAPI document describes {method} {path}"),
                None => write!(f, "no OpenAPI document describes server path {path}"),
            },
            Self::AmbiguousEndpoint {
                method,
                path,
                documents,
            } => {
                let method = method.as_deref().unwrap_or("ANY");
                write!(f, "{method} {path} is described by")?;
                for document in documents {
                    write!(f, " `{}`", document.display())?;
                }
                Ok(())
            }
            Self::Projection { document, source } => {
                write!(
                    f,
                    "OpenAPI projection of `{}` failed: {source}",
                    document.display()
                )
            }
            Self::ConflictingType {
                name,
                first,
                second,
            } => write!(
                f,
                "OpenAPI type `{name}` differs between `{}` and `{}`",
                first.display(),
                second.display()
            ),
            Self::AuthoredTypeConflict { name, source } => write!(
                f,
                "OpenAPI type `{name}` is already authored in `{}`; generated types never overwrite or shadow authored declarations",
                source.display()
            ),
            Self::UnrepresentableName { document, name } => write!(
                f,
                "OpenAPI name `{name}` in `{}` is not a Rastok identifier",
                document.display()
            ),
            Self::UnrepresentableType { document, ty } => write!(
                f,
                "OpenAPI type `{ty}` from `{}` has no Rastok source spelling",
                document.display()
            ),
            Self::Stale { path } => write!(
                f,
                "`{}` is missing or stale; run `rastok --resolve-open-api {}`",
                path.display(),
                path.parent().unwrap_or_else(|| Path::new(".")).display()
            ),
        }
    }
}

impl std::error::Error for ResolveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Projection { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct Endpoint {
    method: Option<String>,
    path: String,
}

struct Specification {
    path: PathBuf,
    document: serde_json::Value,
    json: String,
}

enum TextAttribute {
    Absent,
    Constant(String),
    Dynamic,
}

/// Discovers project endpoints and OpenAPI documents, then generates exact
/// Rastok type aliases.
///
/// # Errors
///
/// Refuses I/O failures, dynamic endpoints, ambiguous specification matches,
/// lossy schema projections, name conflicts, and stale check-mode output.
pub fn resolve_project(root: &Path, mode: ResolveMode) -> Result<ResolveReport, ResolveError> {
    let root = fs::canonicalize(root).map_err(|source| ResolveError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let mut files = Vec::new();
    collect_files(&root, &mut files)?;
    files.sort();

    let catalogue = crate::connector::contract::ContractCatalog::try_m0().map_err(|error| {
        ResolveError::Source {
            path: root.clone(),
            detail: format!("the standard contract catalogue is invalid: {error}"),
        }
    })?;
    let mut endpoints = BTreeSet::new();
    let mut authored_types = BTreeMap::new();
    for (index, path) in files
        .iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "rkg"))
        .enumerate()
    {
        if path.file_name().is_some_and(|name| name == GENERATED_FILE) {
            continue;
        }
        let text = read_text(path)?;
        let source_id = SourceId::new(u32::try_from(index).unwrap_or(u32::MAX));
        let mut bag = Bag::new();
        let tree = crate::compiler::parser::parse(source_id, &text, &catalogue, &mut bag);
        if bag.has_errors() {
            let detail = bag
                .diagnostics()
                .first()
                .map(ToString::to_string)
                .unwrap_or_else(|| String::from("the source did not parse"));
            return Err(ResolveError::Source {
                path: path.clone(),
                detail,
            });
        }
        for declaration in &tree.decls {
            if let TopDecl::Type(declaration) = declaration {
                authored_types
                    .entry(declaration.name.text.clone())
                    .or_insert_with(|| path.clone());
            }
        }
        collect_endpoints(&tree, path, &mut endpoints)?;
    }
    if endpoints.is_empty() {
        return Err(ResolveError::NoEndpoints);
    }

    let mut specifications = Vec::new();
    for path in files.iter().filter(|path| is_schema_document(path)) {
        if let Some(specification) = read_specification(path)? {
            specifications.push(specification);
        }
    }

    let mut selected = BTreeSet::new();
    for endpoint in &endpoints {
        let matches = specifications
            .iter()
            .enumerate()
            .filter_map(|(index, specification)| {
                specification_describes(&specification.document, endpoint).then_some(index)
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => {
                return Err(ResolveError::MissingEndpoint {
                    method: endpoint.method.clone(),
                    path: endpoint.path.clone(),
                });
            }
            [index] => {
                selected.insert(*index);
            }
            _ => {
                return Err(ResolveError::AmbiguousEndpoint {
                    method: endpoint.method.clone(),
                    path: endpoint.path.clone(),
                    documents: matches
                        .iter()
                        .map(|index| specifications[*index].path.clone())
                        .collect(),
                });
            }
        }
    }

    let mut types = BTreeMap::<String, (DataType, PathBuf)>::new();
    for index in &selected {
        let specification = &specifications[*index];
        let projected = crate::schema_projection::openapi::project_components(&specification.json)
            .map_err(|source| ResolveError::Projection {
                document: specification.path.clone(),
                source,
            })?;
        for (name, ty) in projected {
            if let Some(source) = authored_types.get(&name) {
                return Err(ResolveError::AuthoredTypeConflict {
                    name,
                    source: source.clone(),
                });
            }
            if !source_type_name(&name) {
                return Err(ResolveError::UnrepresentableName {
                    document: specification.path.clone(),
                    name,
                });
            }
            validate_member_names(&ty, &specification.path)?;
            if let Some((existing, first)) = types.get(&name) {
                if existing != &ty {
                    return Err(ResolveError::ConflictingType {
                        name,
                        first: first.clone(),
                        second: specification.path.clone(),
                    });
                }
            } else {
                types.insert(name, (ty, specification.path.clone()));
            }
        }
    }

    let selected_paths = selected
        .iter()
        .map(|index| relative_path(&root, &specifications[*index].path))
        .collect::<Vec<_>>();
    let generated = render_generated(&selected_paths, &types)?;
    let output = root.join(GENERATED_FILE);
    let current = match fs::read_to_string(&output) {
        Ok(current) => Some(current),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(ResolveError::Io {
                path: output,
                source,
            });
        }
    };
    let changed = current.as_deref() != Some(generated.as_str());
    match mode {
        ResolveMode::Check if changed => return Err(ResolveError::Stale { path: output }),
        ResolveMode::Check => {}
        ResolveMode::Write if changed => write_atomic(&output, generated.as_bytes())?,
        ResolveMode::Write => {}
    }
    Ok(ResolveReport {
        output,
        endpoints: endpoints.len(),
        specifications: selected.len(),
        types: types.len(),
        changed,
    })
}

fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), ResolveError> {
    let mut entries = fs::read_dir(directory)
        .map_err(|source| ResolveError::Io {
            path: directory.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| ResolveError::Io {
            path: directory.to_path_buf(),
            source,
        })?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        if matches!(name.to_str(), Some(".git" | "target")) {
            continue;
        }
        let kind = entry.file_type().map_err(|source| ResolveError::Io {
            path: path.clone(),
            source,
        })?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            collect_files(&path, files)?;
        } else if kind.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn read_text(path: &Path) -> Result<String, ResolveError> {
    fs::read_to_string(path).map_err(|source| ResolveError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn collect_endpoints(
    tree: &SyntaxTree,
    source: &Path,
    endpoints: &mut BTreeSet<Endpoint>,
) -> Result<(), ResolveError> {
    for declaration in &tree.decls {
        if let TopDecl::Graph(graph) = declaration {
            collect_graph_endpoints(graph, source, endpoints)?;
        }
    }
    Ok(())
}

fn collect_graph_endpoints(
    graph: &GraphDecl,
    source: &Path,
    endpoints: &mut BTreeSet<Endpoint>,
) -> Result<(), ResolveError> {
    for item in &graph.body {
        match item {
            GraphItem::Connector(connector) => {
                if connector
                    .annotation
                    .symbols
                    .iter()
                    .any(|symbol| symbol.text == "HttpClient")
                {
                    let url = match connector_text(connector, "url") {
                        TextAttribute::Constant(url) => url,
                        TextAttribute::Absent | TextAttribute::Dynamic => {
                            return Err(ResolveError::DynamicEndpoint {
                                path: source.to_path_buf(),
                                connector: connector.name.text.clone(),
                                field: "url",
                            });
                        }
                    };
                    let method = match connector_text(connector, "method") {
                        TextAttribute::Absent => String::from("GET"),
                        TextAttribute::Constant(method) => method.to_ascii_uppercase(),
                        TextAttribute::Dynamic => {
                            return Err(ResolveError::DynamicEndpoint {
                                path: source.to_path_buf(),
                                connector: connector.name.text.clone(),
                                field: "method",
                            });
                        }
                    };
                    endpoints.insert(Endpoint {
                        method: Some(method),
                        path: url_path(&url).ok_or_else(|| ResolveError::DynamicEndpoint {
                            path: source.to_path_buf(),
                            connector: connector.name.text.clone(),
                            field: "url",
                        })?,
                    });
                } else if connector
                    .annotation
                    .symbols
                    .iter()
                    .any(|symbol| symbol.text == "HttpServer")
                {
                    let path = match connector_text(connector, "path") {
                        TextAttribute::Absent => String::from("/"),
                        TextAttribute::Constant(path) => path,
                        TextAttribute::Dynamic => {
                            return Err(ResolveError::DynamicEndpoint {
                                path: source.to_path_buf(),
                                connector: connector.name.text.clone(),
                                field: "path",
                            });
                        }
                    };
                    if !path.starts_with('/') {
                        return Err(ResolveError::DynamicEndpoint {
                            path: source.to_path_buf(),
                            connector: connector.name.text.clone(),
                            field: "path",
                        });
                    }
                    endpoints.insert(Endpoint { method: None, path });
                }
            }
            GraphItem::Graph(child) => collect_graph_endpoints(child, source, endpoints)?,
            _ => {}
        }
    }
    Ok(())
}

fn connector_text(connector: &ConnectorDecl, name: &str) -> TextAttribute {
    let Some(value) = connector.body.iter().find_map(|item| {
        let OperatorItem::Attribute(attribute) = item else {
            return None;
        };
        let [segment] = attribute.target.segments.as_slice() else {
            return None;
        };
        (segment.text == name).then_some(&attribute.value)
    }) else {
        return TextAttribute::Absent;
    };
    constant_text(value).map_or(TextAttribute::Dynamic, TextAttribute::Constant)
}

fn constant_text(value: &SynValue) -> Option<String> {
    let SynValue::Expr(source) = value else {
        return None;
    };
    let tree = expr::compile(&source.text, &Signature::new()).ok()?;
    match expr::const_eval(&tree, &BTreeMap::new()).ok()? {
        Value::Text(value) => Some(value),
        _ => None,
    }
}

fn url_path(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let path = rest.find('/').map_or("/", |index| &rest[index..]);
    let end = path.find(['?', '#']).unwrap_or(path.len());
    Some(path[..end].to_string())
}

fn is_schema_document(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("json" | "yaml" | "yml")
    )
}

fn read_specification(path: &Path) -> Result<Option<Specification>, ResolveError> {
    let text = read_text(path)?;
    let document = match path.extension().and_then(|extension| extension.to_str()) {
        Some("json") => serde_json::from_str::<serde_json::Value>(&text).ok(),
        Some("yaml" | "yml") => serde_yaml_ng::from_str::<serde_json::Value>(&text).ok(),
        _ => None,
    };
    let Some(document) = document else {
        return Ok(None);
    };
    if !document
        .get("openapi")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|version| version.starts_with("3.0.") || version.starts_with("3.1."))
    {
        return Ok(None);
    }
    let json = serde_json::to_string(&document).map_err(|source| ResolveError::Source {
        path: path.to_path_buf(),
        detail: source.to_string(),
    })?;
    Ok(Some(Specification {
        path: path.to_path_buf(),
        document,
        json,
    }))
}

fn specification_describes(document: &serde_json::Value, endpoint: &Endpoint) -> bool {
    let Some(path) = document
        .get("paths")
        .and_then(|paths| paths.get(&endpoint.path))
        .and_then(serde_json::Value::as_object)
    else {
        return false;
    };
    const METHODS: &[&str] = &[
        "get", "put", "post", "delete", "options", "head", "patch", "trace",
    ];
    match &endpoint.method {
        Some(method) => path.contains_key(&method.to_ascii_lowercase()),
        None => path.keys().any(|key| METHODS.contains(&key.as_str())),
    }
}

fn source_word(name: &str) -> bool {
    !name.is_empty()
        && !name.as_bytes()[0].is_ascii_digit()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn source_type_name(name: &str) -> bool {
    source_word(name)
        && !matches!(name, "true" | "false" | "null" | "nothing")
        && !crate::compiler::parser::is_builtin_type_name(name)
        && !crate::stdlib::STANDARD_TYPES
            .iter()
            .any(|(standard, _)| *standard == name)
}

fn validate_member_names(ty: &DataType, document: &Path) -> Result<(), ResolveError> {
    let mut stack = vec![ty.clone()];
    while let Some(ty) = stack.pop() {
        match &ty {
            DataType::Struct(structure) => {
                for field in structure.fields() {
                    if !source_word(field.name()) {
                        return Err(ResolveError::UnrepresentableName {
                            document: document.to_path_buf(),
                            name: field.name().to_string(),
                        });
                    }
                    stack.push(field.ty().clone());
                }
            }
            DataType::Map(map) => {
                stack.push(map.key().clone());
                stack.push(map.value().clone());
            }
            DataType::Array(array) => stack.push(array.element().clone()),
            DataType::Enum(enumeration) => {
                if let Some(symbol) = enumeration
                    .symbols()
                    .iter()
                    .find(|symbol| !source_word(symbol))
                {
                    return Err(ResolveError::UnrepresentableName {
                        document: document.to_path_buf(),
                        name: symbol.clone(),
                    });
                }
            }
            DataType::StructuralUnion(union) => stack.extend(union.branches().iter().cloned()),
            DataType::TaggedUnion(union) => {
                for case in union.cases() {
                    if !source_word(case.name()) {
                        return Err(ResolveError::UnrepresentableName {
                            document: document.to_path_buf(),
                            name: case.name().to_string(),
                        });
                    }
                    stack.push(DataType::Struct(case.payload().clone()));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn render_generated(
    documents: &[String],
    types: &BTreeMap<String, (DataType, PathBuf)>,
) -> Result<String, ResolveError> {
    let mut output = String::from(
        "// Generated by `rastok --resolve-open-api`; do not edit.\n// OpenAPI sources:\n",
    );
    for document in documents {
        output.push_str("// - ");
        output.push_str(document);
        output.push('\n');
    }
    output.push('\n');
    for (name, (ty, document)) in types {
        output.push_str("type ");
        output.push_str(name);
        output.push_str(" = ");
        render_type(ty, &mut output).map_err(|ty| ResolveError::UnrepresentableType {
            document: document.clone(),
            ty,
        })?;
        output.push_str("\n\n");
    }
    Ok(output)
}

fn render_type(ty: &DataType, output: &mut String) -> Result<(), String> {
    match ty {
        DataType::Null => output.push_str("null"),
        DataType::Bool => output.push_str("bool"),
        DataType::Integer(IntegerType::Signed(IntegerWidth::Bits32)) => output.push_str("int"),
        DataType::Integer(IntegerType::Signed(IntegerWidth::Bits64)) => output.push_str("long"),
        DataType::Integer(integer) => return Err(format!("{integer:?}")),
        DataType::Float(FloatType::Binary32) => output.push_str("float"),
        DataType::Float(FloatType::Binary64) => output.push_str("double"),
        DataType::Float(float) => return Err(format!("{float:?}")),
        DataType::Decimal(decimal) => {
            output.push_str(&format!(
                "decimal({},{})",
                decimal.precision(),
                decimal.scale()
            ));
        }
        DataType::Number => output.push_str("number"),
        DataType::Text => output.push_str("text"),
        DataType::MediaType => output.push_str("media_type"),
        DataType::Bytes(BinaryType::Variable) => output.push_str("bytes"),
        DataType::Bytes(BinaryType::Fixed(length)) => {
            return Err(format!("bytes({})", length.get()));
        }
        DataType::Temporal(TemporalType::Date(DateUnit::Day)) => output.push_str("date"),
        DataType::Temporal(TemporalType::Time(TimeUnit::Nanosecond)) => output.push_str("time"),
        DataType::Temporal(TemporalType::Timestamp {
            unit: TimeUnit::Nanosecond,
            zone: TimeZone::Utc,
        }) => {
            output.push_str("timestamp");
        }
        DataType::Temporal(TemporalType::TimestampNtz(TimeUnit::Nanosecond)) => {
            output.push_str("timestamp_ntz");
        }
        DataType::Temporal(TemporalType::Duration(TimeUnit::Nanosecond)) => {
            output.push_str("duration")
        }
        DataType::Temporal(other) => return Err(format!("{other:?}")),
        DataType::Uuid => output.push_str("uuid"),
        DataType::Struct(structure) => {
            output.push_str("struct {");
            for (index, field) in structure.fields().iter().enumerate() {
                if index == 0 {
                    output.push(' ');
                } else {
                    output.push_str(", ");
                }
                output.push_str(field.name());
                output.push_str(": ");
                render_type(field.ty(), output)?;
            }
            output.push_str(" }");
        }
        DataType::Map(map) => {
            output.push_str("map<");
            render_type(map.key(), output)?;
            output.push_str(", ");
            render_type(map.value(), output)?;
            output.push('>');
        }
        DataType::Array(array) if array.length().is_none() => {
            output.push_str("array<");
            render_type(array.element(), output)?;
            output.push('>');
        }
        DataType::Array(array) => return Err(format!("fixed array length {:?}", array.length())),
        DataType::Enum(enumeration) => {
            output.push_str("enum {");
            for (index, symbol) in enumeration.symbols().iter().enumerate() {
                if index == 0 {
                    output.push(' ');
                } else {
                    output.push_str(", ");
                }
                output.push_str(symbol);
            }
            output.push_str(" }");
        }
        DataType::StructuralUnion(union) => {
            for (index, branch) in union.branches().iter().enumerate() {
                if index > 0 {
                    output.push_str(" | ");
                }
                render_type(branch, output)?;
            }
        }
        DataType::TaggedUnion(union) => {
            output.push_str("union {");
            for (case_index, case) in union.cases().iter().enumerate() {
                if case_index == 0 {
                    output.push(' ');
                } else {
                    output.push_str(", ");
                }
                output.push_str(case.name());
                if !case.payload().fields().is_empty() {
                    output.push_str(" {");
                    for (field_index, field) in case.payload().fields().iter().enumerate() {
                        if field_index == 0 {
                            output.push(' ');
                        } else {
                            output.push_str(", ");
                        }
                        output.push_str(field.name());
                        output.push_str(": ");
                        render_type(field.ty(), output)?;
                    }
                    output.push_str(" }");
                }
            }
            output.push_str(" }");
        }
        DataType::Variant => output.push_str("variant"),
    }
    Ok(())
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ResolveError> {
    let temporary = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("openapi"),
        std::process::id()
    ));
    let write = || -> Result<(), std::io::Error> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        #[cfg(windows)]
        if path.exists() {
            fs::remove_file(path)?;
        }
        fs::rename(&temporary, path)
    };
    if let Err(source) = write() {
        let _ = fs::remove_file(&temporary);
        return Err(ResolveError::Io {
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static PROJECT: AtomicU64 = AtomicU64::new(0);

    fn project() -> PathBuf {
        let index = PROJECT.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("rastok-openapi-{}-{index}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn project_resolution_discovers_endpoints_and_updates_one_generated_source() {
        let root = project();
        fs::create_dir_all(&root).expect("create project root");
        fs::write(
            root.join("app.rkg"),
            r#"graph demo : App {
              connector payment : HttpClient {
                url = "https://api.example.test/payments"
                method = "POST"
                timeout = duration("1s")
                codec = json
                idempotent = true
                behavior = deterministic
              }
            }"#,
        )
        .expect("write Rastok source");
        fs::write(
            root.join("openapi.yaml"),
            r#"openapi: 3.1.0
paths:
  /payments:
    post:
      responses: {}
components:
  schemas:
    Payment:
      type: object
      additionalProperties: false
      required: [amount, content_type]
      properties:
        amount: { type: number }
        content_type: { type: string, format: media-type }
"#,
        )
        .expect("write OpenAPI source");

        let report = resolve_project(&root, ResolveMode::Write).expect("resolve project");
        assert_eq!(
            (report.endpoints, report.specifications, report.types),
            (1, 1, 1)
        );
        assert!(report.changed);
        let generated = fs::read_to_string(root.join(GENERATED_FILE)).expect("read generated");
        assert!(
            generated
                .contains("type Payment = struct { amount: number, content_type: media_type }")
        );
        let catalogue =
            crate::connector::contract::ContractCatalog::try_m0().expect("catalogue holds");
        let mut bag = Bag::new();
        let _ = crate::compiler::parser::parse(SourceId::new(0), &generated, &catalogue, &mut bag);
        assert!(
            !bag.has_errors(),
            "the generated type source parses through the real host grammar: {:?}",
            bag.diagnostics()
        );
        assert!(
            !resolve_project(&root, ResolveMode::Write)
                .expect("second resolution")
                .changed
        );
        assert!(resolve_project(&root, ResolveMode::Check).is_ok());
        fs::write(root.join(GENERATED_FILE), "// stale\n").expect("make output stale");
        assert!(matches!(
            resolve_project(&root, ResolveMode::Check),
            Err(ResolveError::Stale { .. })
        ));
        fs::remove_dir_all(&root).expect("remove project root");
    }

    #[test]
    fn a_standard_type_name_refuses_before_writing_generated_source() {
        let root = project();
        fs::create_dir_all(&root).expect("create project root");
        fs::write(
            root.join("app.rkg"),
            r#"graph g : App {
              connector remote : HttpClient {
                url = "https://example.test/resource"
                timeout = duration("1s")
                codec = json
                idempotent = true
                behavior = deterministic
              }
            }"#,
        )
        .expect("write source");
        for (name, _) in crate::stdlib::STANDARD_TYPES {
            fs::write(
                root.join("openapi.json"),
                serde_json::json!({
                    "openapi": "3.1.0",
                    "paths": { "/resource": { "get": { "responses": {} } } },
                    "components": { "schemas": { (*name): { "type": "string" } } }
                })
                .to_string(),
            )
            .expect("write schema");
            assert!(matches!(resolve_project(&root, ResolveMode::Write),
                Err(ResolveError::UnrepresentableName { name: refused, .. }) if refused == *name));
            assert!(
                !root.join(GENERATED_FILE).exists(),
                "no invalid declaration is written"
            );
        }
        fs::remove_dir_all(root).expect("remove test project");
    }

    #[test]
    fn a_written_dynamic_endpoint_field_never_falls_back_to_its_default() {
        for (attribute, value) in [("method", "data"), ("path", "data")] {
            let root = project();
            fs::create_dir_all(&root).expect("create project root");
            let connector = if attribute == "method" {
                format!(
                    "connector endpoint : HttpClient {{ url = \"https://api.example.test/items\"; method = {value} }}"
                )
            } else {
                format!("connector endpoint : HttpServer {{ path = {value} }}")
            };
            fs::write(
                root.join("app.rkg"),
                format!("graph demo : App {{ {connector} }}"),
            )
            .expect("write Rastok source");
            assert!(matches!(
                resolve_project(&root, ResolveMode::Write),
                Err(ResolveError::DynamicEndpoint { field, .. }) if field == attribute
            ));
            fs::remove_dir_all(&root).expect("remove project root");
        }
    }

    #[test]
    fn two_documents_claiming_one_endpoint_refuse_in_path_order() {
        let root = project();
        fs::create_dir_all(&root).expect("create project root");
        fs::write(
            root.join("app.rkg"),
            "graph demo : App { connector endpoint : HttpServer { path = \"/items\" } }",
        )
        .expect("write Rastok source");
        let specification = r#"openapi: 3.1.0
paths:
  /items:
    get:
      responses: {}
"#;
        fs::write(root.join("b.yaml"), specification).expect("write second specification");
        fs::write(root.join("a.yaml"), specification).expect("write first specification");
        let canonical_root = fs::canonicalize(&root).expect("canonical project root");

        let Err(ResolveError::AmbiguousEndpoint { documents, .. }) =
            resolve_project(&root, ResolveMode::Write)
        else {
            panic!("two matching specifications must refuse")
        };
        assert_eq!(
            documents,
            [canonical_root.join("a.yaml"), canonical_root.join("b.yaml")],
            "filesystem enumeration cannot choose the diagnostic order"
        );
        fs::remove_dir_all(&root).expect("remove project root");
    }

    #[test]
    fn generated_components_never_shadow_an_authored_type() {
        let root = project();
        fs::create_dir_all(&root).expect("create project root");
        fs::write(
            root.join("app.rkg"),
            "type Payment = text\ngraph demo : App { connector endpoint : HttpServer { path = \"/payments\" } }",
        )
        .expect("write Rastok source");
        fs::write(
            root.join("openapi.yaml"),
            r#"openapi: 3.1.0
paths:
  /payments:
    get:
      responses: {}
components:
  schemas:
    Payment: { type: string }
"#,
        )
        .expect("write specification");

        assert!(matches!(
            resolve_project(&root, ResolveMode::Write),
            Err(ResolveError::AuthoredTypeConflict { name, .. }) if name == "Payment"
        ));
        fs::remove_dir_all(&root).expect("remove project root");
    }
}
