//! Positioned variables are summaries of the already redacted selected frame.

use super::*;
use crate::compiler::lexer::{Lexer, TokenKind};
use crate::diagnostics::{Bag, SourceId};

fn rows(value: &JsonValue) -> &[JsonValue] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn protected(value: &JsonValue) -> bool {
    match value {
        JsonValue::Object(fields) => {
            fields.get("redacted") == Some(&json!(true)) || fields.values().any(protected)
        }
        JsonValue::Array(values) => values.iter().any(protected),
        _ => false,
    }
}

fn summary(kind: &str, value: Option<&JsonValue>, tab: &str, owner: &str) -> JsonValue {
    let redacted = value.is_some_and(protected);
    let display = |value: &JsonValue| {
        if value["redacted"] == true {
            "[REDACTED]".to_owned()
        } else {
            value.get("content").unwrap_or(value).to_string()
        }
    };
    let text = match value {
        None => "Value unavailable at this transition".to_owned(),
        Some(value) if value.get("data").is_some() => format!(
            "DATA {} · META app {} · ext {}",
            display(&value["data"]),
            display(&value["meta"]["app"]),
            display(&value["meta"]["ext"])
        ),
        Some(value) if value.get("reference").is_some() && value.get("state").is_some() => format!(
            "{} · {} · state {}",
            value["label"].as_str().unwrap_or("Instance"),
            value["phase"].as_str().unwrap_or("unavailable"),
            display(&value["state"])
        ),
        Some(value) => display(value),
    };
    let short: String = text.chars().take(220).collect();
    let rendering = format!(
        "{short}{}{}",
        if text.chars().count() > 220 {
            "…"
        } else {
            ""
        },
        if redacted && !short.contains("[REDACTED]") {
            " [REDACTED]"
        } else {
            ""
        }
    );
    json!({"kind": kind, "rendering": rendering, "redacted": redacted,
        "available": value.is_some(), "inspector": {"tab": tab, "node_path": owner,
        "envelope_id": value.and_then(|value| value.get("envelope_id"))}})
}

fn path_value<'a>(mut value: &'a JsonValue, path: &[&str]) -> Option<&'a JsonValue> {
    for name in path {
        if value["redacted"] == true {
            return Some(value);
        }
        if let Some(content) = value.get("content") {
            value = content;
        }
        value = value.get(*name)?;
    }
    Some(value)
}

fn port_value<'a>(
    snapshot: &'a JsonValue,
    owner: &str,
    port: &str,
) -> (Option<&'a JsonValue>, &'static str) {
    let matches =
        |value: &&JsonValue| value["node"].as_str().unwrap_or("") == owner && value["port"] == port;
    let pending: Vec<_> = rows(&snapshot["topology"]["edges"])
        .iter()
        .flat_map(|edge| rows(&edge["mailbox"]))
        .filter(matches)
        .collect();
    if !pending.is_empty() {
        // Multiple instances do not acquire an arbitrary current variable.
        let instances: BTreeSet<_> = pending
            .iter()
            .map(|value| value["instance"].to_string())
            .collect();
        return (
            if instances.len() == 1 {
                pending
                    .iter()
                    .copied()
                    .min_by_key(|value| value["position"].as_u64())
            } else {
                None
            },
            "mailbox",
        );
    }
    let consumed = rows(&snapshot["history"]["records"])
        .iter()
        .rev()
        .flat_map(|record| rows(&record["inputs"]))
        .find(matches);
    if consumed.is_some() {
        return (consumed, "envelope");
    }
    let recorded = ["deliveries", "outputs", "sources"]
        .iter()
        .find_map(|kind| rows(&snapshot["history"][*kind]).iter().rev().find(matches));
    if recorded.is_some() {
        return (recorded, "envelope");
    }
    let draft = &snapshot["scenario"]["current"]["next_injection"];
    (
        (draft["connector"].as_str().unwrap_or("") == owner && draft["port"] == port)
            .then_some(draft),
        "envelope",
    )
}

fn resolve(
    snapshot: &JsonValue,
    plan: &ValidatedExecutionPlan,
    owner: &str,
    name: &str,
) -> JsonValue {
    let path: Vec<_> = name.split('.').collect();
    let root = path[0];
    if matches!(root, "Loop" | "Retry") {
        let kind = if root == "Loop" {
            "loop_runs"
        } else {
            "retry_operations"
        };
        let values: Vec<_> = rows(&snapshot[kind])
            .iter()
            .filter(|value| value["owner"] == owner && value["active"] == true)
            .collect();
        let value = if values.len() == 1 {
            values.first().copied()
        } else {
            None
        };
        let counter = value.map(|value| if root == "Loop" {
            json!({"application": value["application"], "remaining": value["remaining"], "phase": value["phase"], "due_at_ns": value["due_at_ns"]})
        } else { json!({"attempt": value["attempt"], "limit": value["limit"], "phase": value["phase"], "due_at_ns": value["due_at_ns"]}) });
        return summary(
            if root == "Loop" {
                "Loop counter"
            } else {
                "Retry counter"
            },
            counter.as_ref(),
            "instances",
            owner,
        );
    }
    if matches!(root, "timeout" | "lifetime" | "after") {
        let deadlines: Vec<_> = rows(&snapshot["future"])
            .iter()
            .filter(|value| value["owner"] == owner && !value["due_at_ns"].is_null())
            .collect();
        let value = (!deadlines.is_empty()).then(|| json!(deadlines));
        return summary("pending deadline", value.as_ref(), "instances", owner);
    }
    if matches!(root, "secret" | "auth_bearer") {
        return summary(
            "secret binding",
            Some(&json!({"redacted": true})),
            "definition",
            owner,
        );
    }
    if matches!(root, "data" | "meta") {
        let input = rows(&snapshot["history"]["records"])
            .iter()
            .rev()
            .find(|record| record["owner"] == owner && !rows(&record["inputs"]).is_empty())
            .and_then(|record| {
                let inputs = rows(&record["inputs"]);
                let port = record["trigger"]
                    .as_str()
                    .and_then(|trigger| trigger.strip_prefix("p:"));
                inputs
                    .iter()
                    .find(|input| input["port"].as_str() == port)
                    .or_else(|| (inputs.len() == 1).then(|| &inputs[0]))
            });
        let mut result = summary(
            if root == "data" {
                "DATA path"
            } else {
                "META path"
            },
            input.and_then(|input| path_value(&input[root], &path[1..])),
            "envelope",
            owner,
        );
        result["inspector"]["envelope_id"] =
            input.map_or(JsonValue::Null, |input| input["envelope_id"].clone());
        return result;
    }
    let node = plan
        .nodes
        .iter()
        .find(|node| node.path.to_string() == owner);
    if path
        .get(1)
        .is_some_and(|field| matches!(*field, "data" | "meta"))
        && node.is_some_and(|node| node.ports.iter().any(|port| port.port.name() == root))
    {
        let (envelope, tab) = port_value(snapshot, owner, root);
        let mut value = summary(
            "port field",
            envelope.and_then(|value| path_value(&value[path[1]], &path[2..])),
            tab,
            owner,
        );
        value["inspector"]["envelope_id"] =
            envelope.map_or(JsonValue::Null, |value| value["envelope_id"].clone());
        return value;
    }
    if root == "instance" || node.is_some_and(|node| node.vars().iter().any(|var| var.name == root))
    {
        let values: Vec<_> = rows(&snapshot["instances"])
            .iter()
            .filter(|value| value["owner"] == owner)
            .collect();
        let value = (values.len() == 1).then(|| values[0]);
        return summary(
            "instance state",
            value.and_then(|value| {
                if root == "instance" {
                    Some(value)
                } else {
                    path_value(&value["state"], &path)
                }
            }),
            "instances",
            owner,
        );
    }
    // Resolve qualified endpoints in the checked scope, then its parents.
    let mut scopes = vec![owner.to_owned()];
    let mut scope = owner;
    while !scope.is_empty() {
        scope = scope.rsplit_once('.').map_or("", |(parent, _)| parent);
        scopes.push(scope.to_owned());
    }
    for scope in scopes {
        let (target, port) = if path.len() == 1 {
            (scope.clone(), root)
        } else {
            let prefix = path[..path.len() - 1].join(".");
            (
                if prefix == "app" {
                    String::new()
                } else if scope.is_empty() {
                    prefix
                } else {
                    format!("{scope}.{prefix}")
                },
                *path.last().unwrap_or(&""),
            )
        };
        if plan.nodes.iter().any(|node| {
            node.path.to_string() == target
                && node
                    .ports
                    .iter()
                    .any(|candidate| candidate.port.name() == port)
        }) {
            let (value, tab) = port_value(snapshot, &target, port);
            return summary("port envelope", value, tab, &target);
        }
        let node_path = if name == "app" {
            String::new()
        } else if scope.is_empty() {
            name.to_owned()
        } else {
            format!("{scope}.{name}")
        };
        if let Some(node) = plan
            .nodes
            .iter()
            .find(|node| node.path.to_string() == node_path)
        {
            // A shorthand node reference has an ordinary checked `in` port.
            if node.ports.iter().any(|port| port.port.name() == "in") {
                let (value, tab) = port_value(snapshot, &node_path, "in");
                return summary("port envelope", value, tab, &node_path);
            }
        }
    }
    summary("identifier", None, "definition", owner)
}

pub(super) fn tokens(
    snapshot: &JsonValue,
    plan: &ValidatedExecutionPlan,
    definitions: &[SourceDefinition],
) -> JsonValue {
    let mut result = Vec::new();
    for source in rows(&snapshot["sources"]) {
        let Some(code) = source["text"].as_str() else {
            continue;
        };
        let index = source["index"].as_u64().unwrap_or(0) as u32;
        // Exact retained excerpts disambiguate nested declarations on one line.
        // Repeated identical excerpts remain unavailable instead of choosing a scope.
        let scopes: Vec<_> = definitions
            .iter()
            .filter(|definition| definition.source.index() == index)
            .flat_map(|definition| {
                code.match_indices(definition.code.as_str())
                    .filter_map(move |(offset, _)| {
                        let line = code[..offset].bytes().filter(|byte| *byte == b'\n').count() + 1;
                        (line == definition.first_line)
                            .then_some((definition, offset..offset + definition.code.len()))
                    })
            })
            .collect();
        let mut lexer = Lexer::new(SourceId::new(index), code);
        let mut bag = Bag::new();
        let mut tokens = Vec::new();
        let mut trivia_start = 0;
        loop {
            let token = lexer.next(&mut bag);
            // The lexer skips trivia. Inspect only those gaps, using the shared
            // comment recognizer so string contents never become comments.
            let mut offset = trivia_start;
            while offset < token.span.range().start {
                if let Some(comment) = rastok_core::trivia::comment_at(code, offset) {
                    for segment in code[offset..comment.end].split_inclusive('\n') {
                        let text = segment.trim_end_matches(['\r', '\n']);
                        let line = code[..offset].bytes().filter(|byte| *byte == b'\n').count() + 1;
                        let line_start = code[..offset].rfind('\n').map_or(0, |at| at + 1);
                        let column = code[line_start..offset].chars().count() + 1;
                        result.push(json!({"source_index": index, "line": line,
                            "column": column, "end_column": column + text.chars().count(),
                            "kind": "comment", "name": text, "node_path": null, "value": null}));
                        offset += segment.len();
                    }
                } else if let Some(character) = code[offset..].chars().next() {
                    offset += character.len_utf8();
                } else {
                    break;
                }
            }
            trivia_start = token.span.range().end;
            if token.kind == TokenKind::Eof {
                break;
            }
            tokens.push(token);
        }
        let mut cursor = 0;
        while cursor < tokens.len() {
            let token = tokens[cursor];
            let start = token.span.range().start;
            let mut end = token.span.range().end;
            let mut name = token.text.to_owned();
            let annotation = token.text == "@"
                && tokens.get(cursor + 1).is_some_and(|next| {
                    next.kind == TokenKind::Ident && next.span.range().start == end
                });
            if annotation {
                cursor += 1;
                name.push_str(tokens[cursor].text);
                end = tokens[cursor].span.range().end;
            } else if token.kind == TokenKind::Ident {
                while tokens
                    .get(cursor + 1)
                    .is_some_and(|token| token.kind == TokenKind::Dot)
                    && tokens.get(cursor + 2).is_some_and(|token| {
                        token.kind == TokenKind::Ident && !token.newline_before
                    })
                {
                    cursor += 2;
                    name.push('.');
                    name.push_str(tokens[cursor].text);
                    end = tokens[cursor].span.range().end;
                }
            }
            let line = code[..start].bytes().filter(|byte| *byte == b'\n').count() + 1;
            let line_start = code[..start].rfind('\n').map_or(0, |index| index + 1);
            let keyword = matches!(
                name.as_str(),
                "graph"
                    | "operator"
                    | "connector"
                    | "input"
                    | "output"
                    | "on"
                    | "match"
                    | "where"
                    | "var"
                    | "val"
                    | "type"
                    | "struct"
                    | "union"
                    | "lab"
                    | "simulation"
                    | "scenario"
                    | "expect"
                    | "advance"
                    | "mock"
                    | "nothing"
                    | "as"
                    | "is"
                    | "true"
                    | "false"
                    | "App"
            );
            let mut kind = match token.kind {
                _ if annotation => "annotation",
                TokenKind::Ident if keyword => "keyword",
                TokenKind::Ident => "identifier",
                TokenKind::Str => "string",
                TokenKind::Int | TokenKind::Float => "number",
                _ => "punctuation",
            };
            let scope = scopes
                .iter()
                .filter(|(_, range)| range.contains(&start) && end <= range.end)
                .min_by_key(|(_, range)| range.len());
            let ambiguous = scope.is_some_and(|(definition, range)| {
                scopes.iter().any(|(other, other_range)| {
                    other.owner != definition.owner && other_range == range
                })
            });
            let owner = scope
                .map(|(definition, _)| definition.owner.to_string())
                .unwrap_or_default();
            let hover = (kind == "identifier").then(|| {
                if ambiguous {
                    let mut value = summary("identifier", None, "definition", "");
                    value["rendering"] = json!(
                        "Scope unavailable: identical retained declarations share this source line"
                    );
                    value
                } else {
                    resolve(snapshot, plan, &owner, &name)
                }
            });
            if hover
                .as_ref()
                .is_some_and(|value| value["kind"] == "identifier")
                && matches!(
                    name.as_str(),
                    "text"
                        | "long"
                        | "double"
                        | "number"
                        | "media_type"
                        | "null"
                        | "date"
                        | "timestamp_ntz"
                        | "time"
                        | "int"
                        | "uint"
                        | "bool"
                        | "float"
                        | "decimal"
                        | "bytes"
                        | "duration"
                        | "timestamp"
                        | "uuid"
                        | "variant"
                )
            {
                kind = "type";
            }
            // Multiline literals stay ordinary source text; no false one-line span.
            if !code[start..end].contains('\n') {
                result.push(json!({"source_index": index, "line": line,
                    "column": code[line_start..start].chars().count() + 1,
                    "end_column": code[line_start..end].chars().count() + 1,
                    "kind": kind, "name": name, "node_path": if ambiguous { None } else { Some(&owner) }, "value": hover}));
            }
            cursor += 1;
        }
    }
    result.sort_by_key(|token| {
        (
            token["source_index"].as_u64(),
            token["line"].as_u64(),
            token["column"].as_u64(),
        )
    });
    json!(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syntax_projection_distinguishes_comments_annotations_and_string_contents() {
        let program = "graph echo : App { input in: text; output out: text; operator relay { on in { out = data } } in -> relay -> out }";
        let compilation = crate::compiler::compile(
            &[program],
            &crate::connector::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap();
        let code = "// écho\ntext @secret \"https://example.test/path\" /* block\nend */\n";
        let frame = json!({"sources": [{"index": 0, "text": code}]});
        let result = tokens(&frame, &compilation.plan, &[]);
        let tokens = result.as_array().unwrap();
        let comments: Vec<_> = tokens
            .iter()
            .filter(|token| token["kind"] == "comment")
            .collect();
        assert_eq!(comments.len(), 3);
        assert_eq!(comments[0]["name"], "// écho");
        assert_eq!(comments[0]["end_column"], 8);
        assert!(comments.iter().all(|token| token["value"].is_null()));
        assert!(
            tokens
                .iter()
                .any(|token| token["kind"] == "annotation" && token["name"] == "@secret")
        );
        assert!(
            tokens
                .iter()
                .any(|token| token["kind"] == "type" && token["name"] == "text")
        );
        assert!(tokens.iter().any(|token| token["kind"] == "string"
            && token["name"].as_str().unwrap().contains("https://")));
    }

    #[test]
    fn port_summaries_reserve_transport_identifiers_for_the_inspector() {
        let value = json!({"envelope_id": "transport-identifier", "instance": {"address": "transport-address"},
            "data": {"content": {"amount": 7}}, "meta": {"sys": {"id": "transport-identifier"}, "app": {"request": "r"}, "ext": {}}});
        let result = summary("port envelope", Some(&value), "envelope", "partner");
        let rendered = result["rendering"].as_str().unwrap();
        assert!(
            rendered.contains("DATA")
                && rendered.contains("META")
                && rendered.contains("amount")
                && rendered.contains("request")
        );
        assert!(!rendered.contains("transport-"));
        assert_eq!(result["inspector"]["envelope_id"], "transport-identifier");
    }

    #[test]
    fn inline_definitions_keep_identifier_scope_exact() {
        let source = "graph echo : App { input in: text; output out: text; operator a { input in: text; output out: text; on in { out = data } } in -> a -> out }";
        let compilation = crate::compiler::compile(
            &[source],
            &crate::connector::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap();
        let frame = json!({"sources": super::super::source::units(&compilation.plan, &compilation.sources, &compilation.source_definitions)});
        let values = tokens(&frame, &compilation.plan, &compilation.source_definitions);
        let input = values
            .as_array()
            .unwrap()
            .iter()
            .find(|token| token["name"] == "data")
            .unwrap();
        assert_eq!(input["node_path"], "a");
    }

    #[test]
    fn redacted_paths_and_absent_values_have_stable_summaries() {
        let data = json!({"content": {"account": {"redacted": true}}});
        let value = summary(
            "DATA path",
            path_value(&data, &["account", "email"]),
            "envelope",
            "node",
        );
        assert_eq!(value["redacted"], true);
        assert_eq!(value["rendering"], "[REDACTED]");
        let absent = summary(
            "DATA path",
            path_value(&data, &["missing"]),
            "envelope",
            "node",
        );
        assert_eq!(absent["available"], false);
        assert_eq!(absent["inspector"]["tab"], "envelope");
    }
}
