//! Read-only CLI projections of the checked Connector declarations and registered capabilities.
//! No Driver is constructed and no external operation is performed by this catalogue.

use rastok::connector::contract::{
    AttributeSpec, ConnectorContract, ContractCatalog, ContractMode, DefaultValue, EvaluationSite,
    ModeSupport, PortResolutionError, PortTypeRule, Presence, ValueDomain,
};
use rastok::connector::drivers::{DriverManifest, DriverManifestEntry};
use rastok::connector::{ContractId, ExchangeKind, FrameProduction, outbound_call_semantics};
use rastok::datatype::{DataType, TypeField};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;

fn type_view(ty: &DataType) -> Json {
    match ty {
        DataType::Struct(record) => json!({
            "kind": "struct",
            "fields": fields_view(record.fields()),
        }),
        DataType::Map(map) => json!({
            "kind": "map",
            "key": type_view(map.key()),
            "value": type_view(map.value()),
        }),
        DataType::Array(array) => json!({
            "kind": "array",
            "element": type_view(array.element()),
            "length": array.length().map(|length| length.get()),
        }),
        DataType::Enum(symbols) => json!({
            "kind": "enum",
            "symbols": symbols.symbols(),
        }),
        DataType::StructuralUnion(union) => json!({
            "kind": "structural_union",
            "branches": union.branches().iter().map(type_view).collect::<Vec<_>>(),
        }),
        DataType::TaggedUnion(union) => json!({
            "kind": "tagged_union",
            "cases": union.cases().iter().map(|case| json!({
                "name": case.name(),
                "fields": fields_view(case.payload().fields()),
            })).collect::<Vec<_>>(),
        }),
        DataType::Variant => json!({"kind": "variant"}),
        DataType::Null
        | DataType::Bool
        | DataType::Integer(_)
        | DataType::Float(_)
        | DataType::Decimal(_)
        | DataType::Number
        | DataType::Text
        | DataType::MediaType
        | DataType::Bytes(_)
        | DataType::Temporal(_)
        | DataType::Uuid => json!({
            "kind": "scalar",
            "type": ty.to_string(),
        }),
    }
}

fn fields_view(fields: &[TypeField]) -> Vec<Json> {
    fields
        .iter()
        .map(|field| {
            json!({
                "name": field.name(),
                "type": type_view(field.ty()),
            })
        })
        .collect()
}

fn attribute_view(attribute: &AttributeSpec) -> Json {
    let (presence, default) = match &attribute.presence {
        Presence::Required => ("required", Json::Null),
        Presence::Optional => ("optional", Json::Null),
        Presence::Defaulted(DefaultValue::Choice(word)) => ("defaulted", json!({"word": word})),
        Presence::Defaulted(DefaultValue::Expression(source)) => {
            ("defaulted", json!({"expression": source}))
        }
    };
    let domain = match &attribute.domain {
        ValueDomain::Boolean => json!({"kind": "boolean"}),
        ValueDomain::Choice(words) => json!({"kind": "choice", "words": words}),
        ValueDomain::Text => json!({"kind": "text"}),
        ValueDomain::Bytes => json!({"kind": "bytes"}),
        ValueDomain::TextList => json!({"kind": "text_list"}),
        ValueDomain::ChoiceList(words) => json!({"kind": "choice_list", "words": words}),
        ValueDomain::Duration => json!({"kind": "duration"}),
        ValueDomain::ByteSize => json!({"kind": "byte_size"}),
        ValueDomain::TextMap => json!({"kind": "text_map"}),
        ValueDomain::TypedHttpHeaders => json!({"kind": "typed_http_headers"}),
        ValueDomain::CustomHttpHeaders => json!({"kind": "custom_http_headers"}),
        ValueDomain::ExprPath => json!({"kind": "expression_path"}),
        ValueDomain::Struct => json!({"kind": "struct"}),
        ValueDomain::Map => json!({"kind": "map"}),
    };
    let phase = match attribute.latest_site {
        EvaluationSite::Plan => "plan",
        EvaluationSite::Boot => "boot",
        EvaluationSite::Instance => "instance",
        EvaluationSite::Invocation => "invocation",
    };
    json!({
        "name": attribute.name,
        "presence": presence,
        "default": default,
        "domain": domain,
        "latest_phase": phase,
    })
}

fn port_rule_view(rule: PortTypeRule) -> Result<Json, String> {
    match rule {
        PortTypeRule::Fixed(ty) => Ok(json!({
            "kind": "fixed",
            "type": type_view(&ty.resolve().map_err(|error| error.to_string())?),
        })),
        PortTypeRule::SameAs(port) => Ok(json!({"kind": "same_as", "port": port})),
        PortTypeRule::AttributeSelected { attribute, choices }
        | PortTypeRule::JsonOutput { attribute, choices } => {
            let choices = choices
                .iter()
                .map(|choice| {
                    Ok(json!({
                        "word": choice.word,
                        "type": type_view(&choice.ty.resolve().map_err(|error| error.to_string())?),
                    }))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(json!({
                "kind": "attribute_selected",
                "attribute": attribute,
                "choices": choices,
                "json_schema_refinement": matches!(rule, PortTypeRule::JsonOutput { .. }),
            }))
        }
    }
}

fn mode_view(definition: &ConnectorContract, mode: Option<&ContractMode>) -> Result<Json, String> {
    let protocol = mode.and_then(|mode| mode.protocol);
    let id = definition.id;
    let direction = protocol.map_or(definition.roles, |protocol| protocol.roles);
    let exchange = protocol.map_or(definition.exchange, |protocol| protocol.exchange);
    let attributes: Vec<_> = definition
        .common
        .iter()
        .chain(mode.into_iter().flat_map(|mode| mode.attributes))
        .collect();
    if let Some(ContractMode {
        support: ModeSupport::Deferred(reason),
        ..
    }) = mode
    {
        // A declared word is not an executable protocol. In particular File.watch
        // must not advertise the placeholder port types as a promised future shape.
        return Ok(json!({
            "word": mode.map(|mode| mode.word),
            "contract": id,
            "direction": Json::Null,
            "exchange": Json::Null,
            "checker": {"available": false, "reason": reason},
            "attributes": attributes.into_iter().map(attribute_view).collect::<Vec<_>>(),
            "ports": Json::Null,
            "outbound_call_cardinality": Json::Null,
            "interaction": Json::Null,
        }));
    }
    let mut words = BTreeMap::new();
    for attribute in &attributes {
        if let Presence::Defaulted(DefaultValue::Choice(word)) = attribute.presence {
            words.insert(attribute.name.to_owned(), word.to_owned());
        }
    }
    if let (Some(selector), Some(mode)) = (&definition.selector, mode) {
        words.insert(selector.attribute.to_owned(), mode.word.to_owned());
    }
    let port_views = definition
        .ports
        .iter()
        .map(|declared| {
            let (ty, required) = match definition.port_type(declared.name, |attribute| {
                words.get(attribute).map(String::as_str)
            }) {
                Ok(ty) => (Some(ty), None),
                Err(PortResolutionError::MissingAttribute(attribute)) => (None, Some(attribute)),
                Err(error) => return Err(error.to_string()),
            };
            Ok(json!({
                "name": declared.name,
                "direction": match declared.direction {
                    rastok::node::PortDirection::Input => "input",
                    rastok::node::PortDirection::Output => "output",
                },
                "type_rule": port_rule_view(declared.ty)?,
                "default_type": ty.as_ref().map(type_view),
                "type_requires": required,
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let semantics = outbound_call_semantics(
        &ContractId::parse(id).map_err(|error| error.to_string())?,
        direction,
        exchange,
        mode.map(|mode| mode.word),
    );
    let cardinality = semantics.map(|semantics| json!({
        "request": {"port": semantics.request(), "count": 1},
        "terminal": {"port": semantics.terminal(), "count": 1, "after_frames": true},
        "frames": semantics.frame_ports().iter().map(|(port, production)| {
            let production = match production {
                FrameProduction::Never => json!({"count": "never"}),
                FrameProduction::ZeroOrMore => json!({"count": "zero_or_more"}),
                FrameProduction::ExactlyOneOn(cases) => json!({"count": "one_on", "terminal_cases": cases}),
            };
            json!({"port": port, "production": production})
        }).collect::<Vec<_>>(),
        "terminal_visibility": semantics.terminal_cases().iter().map(|(case, visibility)| {
            json!({"case": case, "visibility": visibility.to_string()})
        }).collect::<Vec<_>>(),
    }));
    let support = match mode.map(|mode| mode.support) {
        Some(ModeSupport::Deferred(reason)) => json!({"available": false, "reason": reason}),
        None | Some(ModeSupport::Available) => json!({"available": true}),
    };
    Ok(json!({
        "word": mode.map(|mode| mode.word),
        "contract": id,
        "direction": direction.to_string(),
        "exchange": exchange.to_string(),
        "checker": support,
        "attributes": attributes.into_iter().map(attribute_view).collect::<Vec<_>>(),
        "ports": port_views,
        "outbound_call_cardinality": cardinality,
        "interaction": match exchange {
            ExchangeKind::Event => json!({"scope": "independent ingress", "terminal_count": 0}),
            ExchangeKind::Call => json!({"scope": "one admitted operation", "frames": "zero_or_more", "terminal_port": "exit", "terminal_count": 1}),
            ExchangeKind::Session => json!({"scope": "one open session", "messages": "zero_or_more", "terminal_port": "exit", "terminal_count": 1}),
        },
    }))
}

fn implementation_view(entry: &DriverManifestEntry) -> Json {
    json!({
        "name": entry.name().as_str(),
        "contract": entry.connector().as_str(),
        "direction": entry.roles().to_string(),
        "exchange": entry.exchange().to_string(),
        "mode_attribute": entry.mode_attribute(),
        "modes": entry.modes(),
    })
}

fn compatible_implementations(
    manifest: &DriverManifest,
    id: &str,
    direction: rastok::connector::ConnectorDirection,
    exchange: rastok::connector::ExchangeKind,
    word: Option<&str>,
) -> Vec<Json> {
    manifest
        .entries()
        .iter()
        .filter(|entry| {
            entry.connector().as_str() == id
                && entry.roles().includes(direction)
                && entry.exchange() == exchange
                && match word {
                    Some(word) => entry.modes().iter().any(|mode| mode == word),
                    None => entry.modes().is_empty(),
                }
        })
        .map(implementation_view)
        .collect()
}

fn catalogue(
    catalogue: &ContractCatalog,
    installed: &DriverManifest,
    lab: &DriverManifest,
) -> Result<Json, String> {
    let contracts = catalogue
        .connectors()
        .map(|definition| {
            let modes: Vec<_> = match &definition.selector {
                Some(selector) => selector.modes.iter().map(Some).collect(),
                None => vec![None],
            };
            let modes = modes
                .into_iter()
                .map(|mode| {
                    let protocol = mode.and_then(|mode| mode.protocol);
                    let id = definition.id;
                    let direction = protocol.map_or(definition.roles, |protocol| protocol.roles);
                    let exchange =
                        protocol.map_or(definition.exchange, |protocol| protocol.exchange);
                    let word = mode.map(|mode| mode.word);
                    let mut view = mode_view(definition, mode)?;
                    view["implementations"] = json!(compatible_implementations(
                        installed, id, direction, exchange, word
                    ));
                    view["lab_models"] = json!(compatible_implementations(
                        lab, id, direction, exchange, word
                    ));
                    // Fixture links and untestable cases come from the same metadata
                    // used by conformance fixtures, not an inferred protocol guarantee.
                    view["conformance"] = conformance_view(definition, mode);
                    view["documentation"] = documentation_view(definition, mode);
                    Ok(view)
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(json!({
                "name": definition.name,
                "contract": definition.id,
                "selector": definition.selector.as_ref().map(|selector| json!({
                    "attribute": selector.attribute,
                    "default": selector.default,
                })),
                "modes": modes,
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(json!({
        "contracts": contracts,
        "declared_names": catalogue.declared_names().collect::<Vec<_>>(),
        "scope": "compiled capabilities, not a deployment or a live probe",
        "deployment_validation": "a listed implementation matches Contract, mode, direction and exchange; preparing a deployment still validates its resolved settings and host capabilities",
        "file_configuration": "physical File requires an explicit --file-root at run time",
        "lab_selection": "scripted models substitute effects; deployment implementation selection does not select Lab models",
        "lab_scope": "manifest entries describe bindings, not support for every authored scenario action; consult each mode's documented limits",
    }))
}

fn documentation_view(definition: &ConnectorContract, mode: Option<&ContractMode>) -> Json {
    let (section, example) = match (definition.name, mode.map(|mode| mode.word)) {
        ("Process", Some("job")) => ("3.3", Some("examples/echo.rkg")),
        ("Process", Some("service")) => ("3.3", Some("examples/agent-dialogue.rkg")),
        ("File", Some("read" | "replace")) => ("3.7", Some("examples/file-pipeline.rkg")),
        ("File", _) => ("3.7", None),
        ("HttpClient", _) => ("3.2", Some("examples/webhook-to-partner.rkg")),
        ("HttpServer", Some("ack")) => ("3.1", Some("examples/webhook-to-partner.rkg")),
        ("HttpServer", Some("respond")) => ("3.1", Some("examples/agent-chat.rkg")),
        ("WebSocketClient", _) => ("3.8", None),
        ("WebSocketServer", _) => ("3.9", Some("examples/agent-gateway.rkg")),
        _ => ("", None),
    };
    json!({
        "specification": "docs/spec/stdlib.md",
        "section": section,
        "example": example,
        "example_gap": example.is_none().then_some("no standalone gallery example for this mode; consult the declared support and related test fixtures"),
        "guide": "docs/architecture/connector-drivers.md",
        "lab_limit": (definition.name == "HttpServer" && mode.is_some_and(|mode| mode.word == "respond"))
            .then_some("bindable as a silent Node beside another simulated flow; authored HttpServer source injection supports ack only, not graph-response simulation"),
    })
}

fn conformance_view(definition: &ConnectorContract, mode: Option<&ContractMode>) -> Json {
    use rastok::connector::drivers::conformance::{
        FILE_READ_UNTESTABLE, HTTP_CLIENT_UNTESTABLE, PROCESS_UNTESTABLE,
    };
    let word = mode.map(|mode| mode.word);
    let (fixture, untestable): (Option<&str>, &[(&str, &str)]) = match (definition.name, word) {
        ("Process", Some("job")) => (
            Some("crates/rastok/tests/conformance_process.rs"),
            PROCESS_UNTESTABLE,
        ),
        ("File", Some("read")) => (
            Some("crates/rastok/tests/conformance_file.rs"),
            FILE_READ_UNTESTABLE,
        ),
        ("HttpClient", None) => (
            Some("crates/rastok/tests/conformance_http_client.rs"),
            HTTP_CLIENT_UNTESTABLE,
        ),
        _ => (None, &[]),
    };
    let related_tests = match definition.name {
        "Process" => vec![
            "crates/rastok/tests/process_service.rs",
            "crates/rastok/src/connector/drivers/process/mod.rs",
        ],
        "File" => vec!["crates/rastok/tests/file_connector.rs"],
        "HttpClient" => vec!["crates/rastok/src/connector/drivers/network/http_client.rs"],
        "HttpServer" => vec![
            "crates/rastok/src/connector/drivers/network/http_server.rs",
            "crates/rastok/tests/lab_http_source.rs",
        ],
        "WebSocketClient" | "WebSocketServer" => {
            vec!["crates/rastok/src/connector/drivers/network/websocket.rs"]
        }
        _ => vec![],
    };
    json!({
        "fixture": fixture,
        "untestable_cases": untestable.iter().map(|(case, reason)| json!({"case": case, "reason": reason})).collect::<Vec<_>>(),
        "related_tests": related_tests,
        "qualification": "fixture inventory, not a claim that checks ran in this invocation",
        "missing_fixture": fixture.is_none().then_some("no dedicated full Contract conformance fixture is declared for this mode; related tests are not a substitute for that qualification"),
    })
}

fn type_text(value: &Json) -> String {
    let fields = |fields: &Json| -> String {
        fields
            .as_array()
            .into_iter()
            .flatten()
            .map(|field| format!("{}: {}", text(&field["name"]), type_text(&field["type"])))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match value["kind"].as_str() {
        Some("scalar") => text(&value["type"]).to_owned(),
        Some("variant") => String::from("variant"),
        Some("struct") => format!("struct {{ {} }}", fields(&value["fields"])),
        Some("map") => format!(
            "map<{}, {}>",
            type_text(&value["key"]),
            type_text(&value["value"])
        ),
        Some("array") => {
            let element = type_text(&value["element"]);
            value["length"].as_u64().map_or_else(
                || format!("array<{element}>"),
                |length| format!("array<{element}, {length}>"),
            )
        }
        Some("enum") => format!(
            "enum {{ {} }}",
            value["symbols"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Json::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Some("structural_union") => value["branches"]
            .as_array()
            .into_iter()
            .flatten()
            .map(type_text)
            .collect::<Vec<_>>()
            .join(" | "),
        Some("tagged_union") => format!(
            "union {{ {} }}",
            value["cases"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|case| {
                    format!("{} {{ {} }}", text(&case["name"]), fields(&case["fields"]))
                })
                .collect::<Vec<_>>()
                .join(" | ")
        ),
        _ => String::from("selected by a required attribute"),
    }
}

fn human(view: &Json) -> String {
    let mut output =
        String::from("Standard Connectors\nCompiled capabilities; no Driver is started.\n\n");
    if let Some(contracts) = view["contracts"].as_array() {
        for contract in contracts {
            output.push_str(&format!(
                "{} — {}\n",
                text(&contract["name"]),
                text(&contract["contract"])
            ));
            if !contract["selector"].is_null() {
                output.push_str(&format!(
                    "  selector: {} (default {})\n",
                    text(&contract["selector"]["attribute"]),
                    text(&contract["selector"]["default"])
                ));
            }
            if let Some(modes) = contract["modes"].as_array() {
                for mode in modes {
                    output.push_str(&format!(
                        "  mode {} — checker {}\n",
                        mode["word"].as_str().unwrap_or("<none>"),
                        if mode["checker"]["available"] == true {
                            "available"
                        } else {
                            "deferred"
                        }
                    ));
                    if let Some(reason) = mode["checker"]["reason"].as_str() {
                        output.push_str(&format!("    reason: {reason}\n"));
                    }
                    if !mode["exchange"].is_null() {
                        output.push_str(&format!(
                            "    {} · {} · {}\n",
                            text(&mode["contract"]),
                            text(&mode["direction"]),
                            text(&mode["exchange"])
                        ));
                        output.push_str(&format!("    interaction: {}\n", mode["interaction"]));
                    }
                    if let Some(ports) = mode["ports"].as_array() {
                        for port in ports {
                            output.push_str(&format!(
                                "    {} {}: {}\n",
                                text(&port["direction"]),
                                text(&port["name"]),
                                type_text(&port["default_type"])
                            ));
                            if let Some(attribute) = port["type_requires"].as_str() {
                                output.push_str(&format!(
                                    "      choose {attribute}; no default is declared\n"
                                ));
                            }
                            let rule = &port["type_rule"];
                            if rule["kind"] == "same_as" {
                                output.push_str(&format!(
                                    "      type follows port {}\n",
                                    text(&rule["port"])
                                ));
                            }
                            if let Some(choices) = rule["choices"].as_array() {
                                for choice in choices {
                                    output.push_str(&format!(
                                        "      {} = {}: {}\n",
                                        text(&rule["attribute"]),
                                        text(&choice["word"]),
                                        type_text(&choice["type"])
                                    ));
                                }
                                if rule["json_schema_refinement"] == true {
                                    output.push_str(
                                        "      JSON output may be refined by a declared schema\n",
                                    );
                                }
                            }
                        }
                    }
                    if !mode["outbound_call_cardinality"].is_null() {
                        output.push_str(&format!(
                            "    Call cardinality: {}\n",
                            mode["outbound_call_cardinality"]
                        ));
                    }
                    if let Some(attributes) = mode["attributes"].as_array() {
                        for attribute in attributes {
                            output.push_str(&format!(
                                "    attribute {}: {} · {} · latest {} · default {}\n",
                                text(&attribute["name"]),
                                attribute["domain"],
                                text(&attribute["presence"]),
                                text(&attribute["latest_phase"]),
                                attribute["default"]
                            ));
                        }
                    }
                    for (key, label) in [
                        ("implementations", "installed Drivers"),
                        ("lab_models", "Lab bindings"),
                    ] {
                        let names: Vec<_> = mode[key]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|entry| entry["name"].as_str())
                            .collect();
                        output.push_str(&format!(
                            "    {label}: {}\n",
                            if names.is_empty() {
                                String::from("none")
                            } else {
                                names.join(", ")
                            }
                        ));
                    }
                    let conformance = &mode["conformance"];
                    output.push_str(&format!(
                        "    conformance fixture: {}\n",
                        text(&conformance["fixture"])
                    ));
                    output.push_str(&format!("      {}\n", text(&conformance["qualification"])));
                    if let Some(reason) = conformance["missing_fixture"].as_str() {
                        output.push_str(&format!("      {reason}\n"));
                    }
                    if let Some(cases) = conformance["untestable_cases"].as_array() {
                        for case in cases {
                            output.push_str(&format!(
                                "      untested {}: {}\n",
                                text(&case["case"]),
                                text(&case["reason"])
                            ));
                        }
                    }
                    if let Some(tests) = conformance["related_tests"].as_array() {
                        for test in tests {
                            output.push_str(&format!("      related test: {}\n", text(test)));
                        }
                    }
                    let docs = &mode["documentation"];
                    output.push_str(&format!(
                        "    specification: {} section {}\n",
                        text(&docs["specification"]),
                        text(&docs["section"])
                    ));
                    output.push_str(&format!("    example: {}\n", text(&docs["example"])));
                    for field in ["example_gap", "lab_limit"] {
                        if let Some(reason) = docs[field].as_str() {
                            output.push_str(&format!("      {reason}\n"));
                        }
                    }
                }
            }
            output.push('\n');
        }
    }
    output.push_str(&format!(
        "Declared names without a Contract: {}\n",
        view["declared_names"]
    ));
    output.push_str(&format!(
        "{}\n{}\n{}\n",
        text(&view["file_configuration"]),
        text(&view["lab_selection"]),
        text(&view["deployment_validation"])
    ));
    output
}

fn text(value: &Json) -> &str {
    value.as_str().unwrap_or("<none>")
}

pub(super) fn run(arguments: &[String]) -> std::process::ExitCode {
    let machine = match arguments {
        [] => false,
        [flag] if flag == "--json" => true,
        _ => {
            eprintln!("usage: rastok connectors [--json]");
            return std::process::ExitCode::from(2);
        }
    };
    let result = (|| {
        let definitions = ContractCatalog::try_m0().map_err(|error| error.to_string())?;
        // Factories describe capabilities here; no Driver is prepared and no
        // file is opened. A physical run must still supply its File root.
        let installed =
            rastok::connector::drivers::hosted_registry(Some(std::path::Path::new(".")))
                .map_err(|error| error.to_string())?
                .manifest()
                .map_err(|error| error.to_string())?;
        let lab = rastok::lab::driver_manifest()?;
        catalogue(&definitions, &installed, &lab)
    })();
    match result {
        Ok(view) => {
            if machine {
                match serde_json::to_string_pretty(&view) {
                    Ok(text) => println!("{text}"),
                    Err(error) => {
                        eprintln!("error: cannot encode Connector catalogue: {error}");
                        return std::process::ExitCode::FAILURE;
                    }
                }
            } else {
                print!("{}", human(&view));
            }
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: cannot describe Connector catalogue: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> Json {
        catalogue(
            &ContractCatalog::try_m0().unwrap(),
            &rastok::connector::drivers::hosted_registry(Some(std::path::Path::new("unused-root")))
                .unwrap()
                .manifest()
                .unwrap(),
            &rastok::lab::driver_manifest().unwrap(),
        )
        .unwrap()
    }

    fn mode<'a>(view: &'a Json, name: &str, word: Option<&str>) -> &'a Json {
        view["contracts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|contract| contract["name"] == name)
            .unwrap()["modes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|mode| mode["word"].as_str() == word)
            .unwrap()
    }

    #[test]
    fn the_catalogue_projects_every_checked_definition_and_resolved_port() {
        let view = view();
        let definitions = ContractCatalog::try_m0().unwrap();
        assert_eq!(
            view["contracts"].as_array().unwrap().len(),
            definitions.connectors().count()
        );
        for definition in definitions.connectors() {
            let modes: Vec<_> = definition.selector.as_ref().map_or(vec![None], |selector| {
                selector.modes.iter().map(Some).collect()
            });
            for declared in modes {
                let actual = mode(&view, definition.name, declared.map(|mode| mode.word));
                assert_eq!(actual["contract"], definition.id);
                let protocol = declared.and_then(|mode| mode.protocol);
                let expected_names: Vec<_> = definition
                    .common
                    .iter()
                    .chain(declared.into_iter().flat_map(|mode| mode.attributes))
                    .map(|attribute| attribute.name)
                    .collect();
                let actual_names: Vec<_> = actual["attributes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|attribute| attribute["name"].as_str().unwrap())
                    .collect();
                assert_eq!(actual_names, expected_names);
                assert_eq!(
                    actual["checker"]["available"],
                    !declared.is_some_and(|mode| matches!(mode.support, ModeSupport::Deferred(_)))
                );
                if actual["checker"]["available"] == true {
                    assert_eq!(
                        actual["direction"],
                        protocol
                            .map_or(definition.roles, |protocol| protocol.roles)
                            .to_string()
                    );
                    assert_eq!(
                        actual["exchange"],
                        protocol
                            .map_or(definition.exchange, |protocol| protocol.exchange)
                            .to_string()
                    );
                    assert_eq!(
                        actual["ports"].as_array().unwrap().len(),
                        definition.ports.len()
                    );
                    let mut words = BTreeMap::new();
                    for attribute in definition
                        .common
                        .iter()
                        .chain(declared.into_iter().flat_map(|mode| mode.attributes))
                    {
                        if let Presence::Defaulted(DefaultValue::Choice(word)) = attribute.presence
                        {
                            words.insert(attribute.name.to_owned(), word.to_owned());
                        }
                    }
                    if let (Some(selector), Some(declared)) = (&definition.selector, declared) {
                        words.insert(selector.attribute.to_owned(), declared.word.to_owned());
                    }
                    for declared_port in definition.ports {
                        let projected = actual["ports"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|port| port["name"] == declared_port.name)
                            .unwrap();
                        match definition.port_type(declared_port.name, |attribute| {
                            words.get(attribute).map(String::as_str)
                        }) {
                            Ok(ty) => {
                                assert_eq!(projected["default_type"], type_view(&ty));
                                assert!(projected["type_requires"].is_null());
                            }
                            Err(PortResolutionError::MissingAttribute(attribute)) => {
                                assert!(projected["default_type"].is_null());
                                assert_eq!(projected["type_requires"], attribute);
                            }
                            Err(error) => panic!("invalid standard port: {error}"),
                        }
                    }
                }
            }
        }
        assert_eq!(
            view["declared_names"],
            json!(["Timer", "Calendar", "Memory"])
        );
        let serialized = serde_json::to_string(&view).unwrap();
        assert_eq!(serde_json::from_str::<Json>(&serialized).unwrap(), view);
    }

    #[test]
    fn required_codec_choices_do_not_acquire_an_invented_default() {
        let view = view();
        let server = mode(&view, "HttpServer", Some("ack"));
        let out = server["ports"]
            .as_array()
            .unwrap()
            .iter()
            .find(|port| port["name"] == "out")
            .unwrap();
        assert!(out["default_type"].is_null());
        assert_eq!(out["type_requires"], "codec");
        assert_eq!(out["type_rule"], json!({"kind": "same_as", "port": "in"}));
        let input = server["ports"]
            .as_array()
            .unwrap()
            .iter()
            .find(|port| port["name"] == "in")
            .unwrap();
        let definition = rastok::connector::stdlib::HTTP_SERVER;
        for choice in input["type_rule"]["choices"].as_array().unwrap() {
            let words = BTreeMap::from([
                (String::from("mode"), String::from("ack")),
                (
                    String::from("codec"),
                    choice["word"].as_str().unwrap().to_owned(),
                ),
            ]);
            let resolved = definition.resolve_ports(&words).unwrap();
            let out = resolved
                .iter()
                .find(|port| port.port.name() == "out")
                .unwrap();
            assert_eq!(choice["type"], type_view(out.ty.as_ref().unwrap()));
        }
        assert!(human(&view).contains("choose codec; no default is declared"));
    }

    #[test]
    fn mode_support_lab_support_and_installed_drivers_are_distinct() {
        let view = view();
        let deferred = mode(&view, "File", Some("watch"));
        assert_eq!(deferred["checker"]["available"], false);
        assert!(
            deferred["checker"]["reason"]
                .as_str()
                .is_some_and(|reason| !reason.is_empty())
        );
        assert!(deferred["ports"].is_null());
        assert!(deferred["exchange"].is_null());
        assert!(deferred["implementations"].as_array().unwrap().is_empty());
        assert!(deferred["lab_models"].as_array().unwrap().is_empty());

        let ack = mode(&view, "HttpServer", Some("ack"));
        assert_eq!(ack["checker"]["available"], true);
        assert_eq!(ack["lab_models"][0]["name"], "lab-http-server");
        let respond = mode(&view, "HttpServer", Some("respond"));
        assert_eq!(respond["checker"]["available"], true);
        assert_eq!(respond["lab_models"][0]["name"], "lab-http-server");
        assert!(
            respond["documentation"]["lab_limit"]
                .as_str()
                .unwrap()
                .contains("ack only")
        );
        assert_eq!(
            respond["implementations"].as_array().unwrap().is_empty(),
            !cfg!(feature = "hosted-http12")
        );

        let job = mode(&view, "Process", Some("job"));
        let service = mode(&view, "Process", Some("service"));
        assert_eq!(job["implementations"][0]["name"], "process-job");
        assert_eq!(service["implementations"][0]["name"], "process-service");
        assert_eq!(job["lab_models"][0]["name"], "lab-call");
        assert_eq!(service["lab_models"][0]["name"], "lab-process-service");
        assert_ne!(job["exchange"], service["exchange"]);
        assert_eq!(job["contract"], service["contract"]);
        assert_eq!(
            job["contract"],
            rastok::connector::stdlib::PROCESS_CONNECTOR_ID
        );
        assert_eq!(job["outbound_call_cardinality"]["terminal"]["count"], 1);
        assert!(service["outbound_call_cardinality"].is_null());
        assert_eq!(service["interaction"]["terminal_count"], 1);
    }

    #[test]
    fn type_rules_terminal_cases_and_untestable_cases_are_visible_in_both_views() {
        let view = view();
        let file = mode(&view, "File", Some("read"));
        assert_eq!(file["implementations"][0]["name"], "file");
        let ports = file["ports"].as_array().unwrap();
        let exit = ports.iter().find(|port| port["name"] == "exit").unwrap();
        let expected = rastok::connector::stdlib::file_exit_type().unwrap();
        assert_eq!(exit["default_type"], type_view(&expected));
        assert_eq!(
            file["conformance"]["untestable_cases"]
                .as_array()
                .unwrap()
                .len(),
            rastok::connector::drivers::conformance::FILE_READ_UNTESTABLE.len()
        );
        let output = human(&view);
        for word in [
            "Process",
            "File",
            "HttpClient",
            "HttpServer",
            "WebSocketClient",
            "WebSocketServer",
            "ServerError",
            "handshake_timeout",
            "DecodeFailed",
            "atomic rename",
            "--file-root",
        ] {
            assert!(output.contains(word), "missing {word}");
        }
        assert!(output.contains("fixture inventory, not a claim that checks ran"));
    }

    #[test]
    fn an_empty_driver_manifest_does_not_hide_the_contract_or_lab_model() {
        let view = catalogue(
            &ContractCatalog::try_m0().unwrap(),
            &DriverManifest::try_new([]).unwrap(),
            &rastok::lab::driver_manifest().unwrap(),
        )
        .unwrap();
        let job = mode(&view, "Process", Some("job"));
        assert_eq!(job["checker"]["available"], true);
        assert!(job["implementations"].as_array().unwrap().is_empty());
        assert!(!job["lab_models"].as_array().unwrap().is_empty());
    }

    #[test]
    fn the_contributor_file_example_compiles_against_the_catalogue() {
        let guide = include_str!("../../../docs/architecture/connector-drivers.md");
        let source = guide
            .split("```rkg\n")
            .nth(1)
            .unwrap()
            .split("```")
            .next()
            .unwrap();
        rastok::compiler::compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    }
}
