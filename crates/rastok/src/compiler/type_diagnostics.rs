//! Bounded type descriptions for source diagnostics. Alias names are
//! presentation only and never change structural equality or the plan.

use crate::datatype::{DataType, StructType};

const DISPLAY_DEPTH: usize = 3;
const DISPLAY_MEMBERS: usize = 4;
const DISPLAY_CHARS: usize = 240;

pub(super) fn describe(ty: &DataType, names: &[(&str, &DataType)]) -> String {
    let text = render(ty, names, DISPLAY_DEPTH);
    if text.chars().count() <= DISPLAY_CHARS {
        text
    } else {
        format!(
            "{}...",
            text.chars().take(DISPLAY_CHARS - 3).collect::<String>()
        )
    }
}

fn render(ty: &DataType, names: &[(&str, &DataType)], depth: usize) -> String {
    // Both callers supply names in sorted order, so equivalent aliases do
    // not make diagnostics depend on declaration order.
    if let Some((name, _)) = names.iter().find(|(_, named)| *named == ty) {
        return (*name).to_owned();
    }
    if depth == 0 {
        return "...".into();
    }
    match ty {
        DataType::Struct(structure) => format!("struct {{ {} }}", fields(structure, names, depth)),
        DataType::TaggedUnion(union) => {
            let mut cases: Vec<_> = union
                .cases()
                .iter()
                .take(DISPLAY_MEMBERS)
                .map(|case| {
                    format!(
                        "{} {{ {} }}",
                        case.name(),
                        fields(case.payload(), names, depth - 1)
                    )
                })
                .collect();
            if union.cases().len() > DISPLAY_MEMBERS {
                cases.push("...".into());
            }
            format!("union {{ {} }}", cases.join(", "))
        }
        DataType::Enum(enumeration) => {
            let mut symbols: Vec<_> = enumeration
                .symbols()
                .iter()
                .take(DISPLAY_MEMBERS)
                .cloned()
                .collect();
            if enumeration.symbols().len() > DISPLAY_MEMBERS {
                symbols.push("...".into());
            }
            format!("enum {{ {} }}", symbols.join(", "))
        }
        DataType::StructuralUnion(union) => {
            let mut branches: Vec<_> = union
                .branches()
                .iter()
                .take(DISPLAY_MEMBERS)
                .map(|ty| render(ty, names, depth - 1))
                .collect();
            if union.branches().len() > DISPLAY_MEMBERS {
                branches.push("...".into());
            }
            branches.join(" | ")
        }
        DataType::Array(array) => match array.length() {
            None => format!("array<{}>", render(array.element(), names, depth - 1)),
            Some(length) => format!(
                "array<{}, {}>",
                render(array.element(), names, depth - 1),
                length.get()
            ),
        },
        DataType::Map(map) => format!(
            "map<{}, {}>",
            render(map.key(), names, depth - 1),
            render(map.value(), names, depth - 1)
        ),
        &DataType::INT => "int".into(),
        &DataType::LONG => "long".into(),
        &DataType::UINT => "uint".into(),
        &DataType::FLOAT => "float".into(),
        &DataType::DOUBLE => "double".into(),
        &DataType::DURATION => "duration".into(),
        &DataType::TIMESTAMP => "timestamp".into(),
        _ => ty.to_string(),
    }
}

fn fields(structure: &StructType, names: &[(&str, &DataType)], depth: usize) -> String {
    if depth == 0 {
        return "...".into();
    }
    let mut fields: Vec<_> = structure
        .fields()
        .iter()
        .take(DISPLAY_MEMBERS)
        .map(|field| format!("{}: {}", field.name(), render(field.ty(), names, depth - 1)))
        .collect();
    if structure.fields().len() > DISPLAY_MEMBERS {
        fields.push("...".into());
    }
    fields.join(", ")
}

/// A suffix explaining the first structural disagreement, without replacing
/// the resolved types used by the checker or the structured diagnostic fields.
pub(super) fn difference(
    found: &DataType,
    expected: &DataType,
    names: &[(&str, &DataType)],
) -> String {
    let left = describe(found, names);
    let right = describe(expected, names);
    let detail = match (found, expected) {
        (DataType::Struct(a), DataType::Struct(b)) => field_difference(a, b, &left, &right, names),
        (DataType::TaggedUnion(a), DataType::TaggedUnion(b)) => {
            let mut absent = Vec::new();
            if let Some(case) = a
                .cases()
                .iter()
                .find(|case| !b.cases().iter().any(|other| other.name() == case.name()))
            {
                absent.push(format!("`{right}` has no case `{}`", case.name()));
            }
            if let Some(case) = b
                .cases()
                .iter()
                .find(|case| !a.cases().iter().any(|other| other.name() == case.name()))
            {
                absent.push(format!("`{left}` has no case `{}`", case.name()));
            }
            if absent.is_empty() {
                a.cases()
                    .iter()
                    .find_map(|case| {
                        let other = b.cases().iter().find(|other| other.name() == case.name())?;
                        let detail =
                            field_difference(case.payload(), other.payload(), &left, &right, names);
                        (!detail.is_empty()).then(|| format!("case `{}`: {detail}", case.name()))
                    })
                    .unwrap_or_default()
            } else {
                absent.join(", and ")
            }
        }
        _ => String::new(),
    };
    if detail.is_empty() {
        detail
    } else {
        format!(": {detail}")
    }
}

fn field_difference(
    a: &StructType,
    b: &StructType,
    left: &str,
    right: &str,
    names: &[(&str, &DataType)],
) -> String {
    let mut absent = Vec::new();
    if let Some(field) = a
        .fields()
        .iter()
        .find(|field| !b.fields().iter().any(|other| other.name() == field.name()))
    {
        absent.push(format!("`{right}` has no field `{}`", field.name()));
    }
    if let Some(field) = b
        .fields()
        .iter()
        .find(|field| !a.fields().iter().any(|other| other.name() == field.name()))
    {
        absent.push(format!("`{left}` has no field `{}`", field.name()));
    }
    if !absent.is_empty() {
        return absent.join(", and ");
    }
    a.fields()
        .iter()
        .find_map(|field| {
            let other = b
                .fields()
                .iter()
                .find(|other| other.name() == field.name())?;
            (field.ty() != other.ty()).then(|| {
                format!(
                    "field `{}` is `{}` in `{left}`, but `{}` in `{right}`",
                    field.name(),
                    describe(field.ty(), names),
                    describe(other.ty(), names),
                )
            })
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datatype::TypeField;

    #[test]
    fn route_type_fields_and_messages_name_structs_and_tagged_cases() {
        for (a, b, detail) in [
            (
                "struct { article: text, hash: text }",
                "struct { article: text, reviewer: text }",
                "`B` has no field `hash`, and `A` has no field `reviewer`",
            ),
            (
                "union { Good { value: text }, Failed {} }",
                "union { Good { value: int }, Failed {} }",
                "case `Good`: field `value` is `text` in `A`, but `int` in `B`",
            ),
            (
                "union { Good {}, Failed {} }",
                "union { Good {}, Rejected {} }",
                "`B` has no case `Failed`, and `A` has no case `Rejected`",
            ),
        ] {
            let source = format!(
                "type A = {a}\ntype B = {b}\ngraph app : App {{ input in: A\n output out: B\n in -> out }}"
            );
            let registry = crate::connector::contract::ContractCatalog::try_m0().unwrap();
            let bag = crate::compiler::compile(&[&source], &registry).unwrap_err();
            let diagnostic = bag
                .diagnostics()
                .iter()
                .find(|d| d.code() == crate::compiler::check::E_PORT_TYPE)
                .unwrap();
            assert!(diagnostic.message().contains(detail), "{bag:?}");
            assert!(diagnostic.fields().contains(&("expected", "B".into())));
            assert!(diagnostic.fields().contains(&("found", "A".into())));
        }
    }

    #[test]
    fn anonymous_type_descriptions_bound_width_depth_and_text() {
        let ty = DataType::Struct(
            StructType::new(
                (0..8)
                    .map(|i| TypeField::new(format!("field{i}"), DataType::INT))
                    .collect(),
            )
            .unwrap(),
        );
        assert_eq!(
            describe(&ty, &[]),
            "struct { field0: int, field1: int, field2: int, field3: int, ... }"
        );
        let mut deep = ty;
        for _ in 0..10 {
            deep = DataType::array(deep);
        }
        assert_eq!(describe(&deep, &[]), "array<array<array<...>>>");
        let name = "名".repeat(300);
        assert_eq!(
            describe(&deep, &[(name.as_str(), &deep)]).chars().count(),
            DISPLAY_CHARS
        );
    }
}
