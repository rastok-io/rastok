//! External-schema projection conformance for the complete B9 profile.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::datatype::{
    ArrayType, BinaryType, ByteLength, DataType, DateUnit, DecimalType, EnumType, FloatType,
    IntegerType, IntegerWidth, IntervalUnit, MapType, StructType, TaggedCase, TaggedUnionType,
    TemporalType, TimeUnit, TimeZone, TypeField,
};
use rastok::schema_projection::arrow::{
    self, ArrowBinaryKind, ArrowDecimalWidth, ArrowField, ArrowTextKind, ArrowType,
    ArrowUnionChild, ArrowUnionMode,
};
use rastok::schema_projection::{LossReason, UnionSelection, avro, json_schema};
use rastok::value::{TaggedUnionValue, Value};

fn structure(fields: impl IntoIterator<Item = (&'static str, DataType)>) -> DataType {
    DataType::Struct(
        StructType::new(
            fields
                .into_iter()
                .map(|(name, ty)| TypeField::new(name, ty))
                .collect(),
        )
        .expect("the fixture has unique fields"),
    )
}

fn optional(ty: DataType) -> DataType {
    ty.optional().expect("the fixture is nullable once")
}

#[test]
fn json_schema_supported_rows_project_to_the_exact_logical_type() {
    let rows = [
        (r#"{"type":"null"}"#, DataType::Null),
        (r#"{"type":"boolean"}"#, DataType::Bool),
        (r#"{"type":"string"}"#, DataType::Text),
        (
            r#"{"type":"integer","minimum":-128,"maximum":127}"#,
            DataType::Integer(IntegerType::Signed(IntegerWidth::Bits8)),
        ),
        (
            r#"{"type":"integer","minimum":0,"maximum":65535}"#,
            DataType::Integer(IntegerType::Unsigned(IntegerWidth::Bits16)),
        ),
        (r#"{"type":"number"}"#, DataType::Number),
        (r#"{"type":["string"]}"#, DataType::Text),
        (
            r#"{"type":"string","enum":["ready","done"]}"#,
            DataType::Enum(
                EnumType::new(vec![String::from("done"), String::from("ready")])
                    .expect("unique enum"),
            ),
        ),
        (
            r#"{"type":"array","items":{"type":"boolean"},"minItems":3,"maxItems":3}"#,
            DataType::Array(ArrayType::fixed(DataType::Bool, 3).expect("valid length")),
        ),
        (
            r#"{"type":"object","properties":{},"additionalProperties":{"type":"string"}}"#,
            DataType::text_map(DataType::Text),
        ),
        (r#"{"type":["null","string"]}"#, optional(DataType::Text)),
    ];

    for (document, expected) in rows {
        assert_eq!(json_schema::project(document), Ok(expected), "{document}");
    }
}

#[test]
fn json_schema_order_and_local_names_are_not_logical_identity() {
    let left = r##"{
      "$defs":{"Name":{"type":"string"}},
      "type":"object",
      "properties":{"age":{"type":"integer","minimum":0,"maximum":255},"name":{"$ref":"#/$defs/Name"}},
      "required":["name","age"],
      "additionalProperties":false
    }"##;
    let right = r##"{
      "additionalProperties":false,
      "required":["age","name"],
      "properties":{"name":{"$ref":"#/$defs/Name"},"age":{"maximum":255,"minimum":0,"type":"integer"}},
      "type":"object",
      "$defs":{"Name":{"type":"string"}}
    }"##;
    let expected = structure([
        (
            "age",
            DataType::Integer(IntegerType::Unsigned(IntegerWidth::Bits8)),
        ),
        ("name", DataType::Text),
    ]);
    assert_eq!(json_schema::project(left), Ok(expected.clone()));
    assert_eq!(json_schema::project(right), Ok(expected));
}

#[test]
fn json_schema_reverse_projection_round_trips_or_refuses_a_named_loss() {
    let exact = [
        DataType::Bool,
        DataType::Integer(IntegerType::Unsigned(IntegerWidth::Bits64)),
        DataType::Number,
        structure([
            ("a", DataType::Text),
            ("z", DataType::array(DataType::Bool)),
        ]),
        DataType::text_map(DataType::Text),
        optional(DataType::Text),
    ];
    for ty in exact {
        let document = json_schema::reverse(&ty).expect("the type has an exact standard schema");
        assert_eq!(json_schema::project(&document), Ok(ty), "{document}");
    }

    let error = json_schema::reverse(&DataType::BYTES).expect_err("bytes need an extension");
    assert_eq!(error.reason(), LossReason::UnsupportedTarget);
    assert_eq!(error.path(), "$");
}

#[test]
fn json_schema_refuses_constraints_cycles_and_ambiguous_unions_at_their_path() {
    let boolean = json_schema::project("true")
        .expect_err("boolean schemas are valid JSON Schema but outside the strict profile");
    assert_eq!(boolean.reason(), LossReason::UnsupportedSource);
    assert_eq!(boolean.path(), "$");

    let empty = json_schema::project(r#"{"type":[]}"#)
        .expect_err("a type array must name at least one type");
    assert_eq!(empty.reason(), LossReason::InvalidSchema);
    assert_eq!(empty.path(), "$.type");

    let narrow_range = json_schema::project(r#"{"type":"integer","minimum":0,"maximum":10}"#)
        .expect_err("a partial machine-width range would be widened");
    assert_eq!(narrow_range.reason(), LossReason::LossyProjection);
    assert_eq!(narrow_range.path(), "$");

    let optional_property = json_schema::project(
        r#"{"type":"object","properties":{"name":{"type":"string"}},"additionalProperties":false}"#,
    )
    .expect_err("missing and null are not one value");
    assert_eq!(optional_property.reason(), LossReason::LossyProjection);
    assert_eq!(optional_property.path(), "$.properties.name");

    let required_map_key = json_schema::project(
        r#"{"type":"object","required":["id"],"additionalProperties":{"type":"string"}}"#,
    )
    .expect_err("a homogeneous map cannot preserve a mandatory key");
    assert_eq!(required_map_key.reason(), LossReason::LossyProjection);
    assert_eq!(required_map_key.path(), "$.required");

    let constrained_type_array = json_schema::project(r#"{"type":["string"],"pattern":"x"}"#)
        .expect_err("a type array cannot hide sibling constraints");
    assert_eq!(
        constrained_type_array.reason(),
        LossReason::UnsupportedSource
    );
    assert_eq!(constrained_type_array.path(), "$.pattern");

    let pattern = json_schema::project(r#"{"type":"string","pattern":"x"}"#)
        .expect_err("pattern is outside the strict profile");
    assert_eq!(pattern.reason(), LossReason::UnsupportedSource);
    assert_eq!(pattern.path(), "$.pattern");

    let cycle = json_schema::project(
        r##"{"$defs":{"Node":{"type":"object","properties":{"next":{"$ref":"#/$defs/Node"}},"required":["next"],"additionalProperties":false}},"$ref":"#/$defs/Node"}"##,
    )
    .expect_err("recursive definitions refuse");
    assert_eq!(cycle.reason(), LossReason::RecursiveSchema);

    let unused_bad_definition = json_schema::project(r#"{"$defs":{"Bad":42},"type":"string"}"#)
        .expect_err("a complete schema cannot hide an invalid unused definition");
    assert_eq!(unused_bad_definition.reason(), LossReason::InvalidSchema);
    assert_eq!(unused_bad_definition.path(), "$.$defs.Bad");

    let bad_pointer =
        json_schema::project(r##"{"$defs":{"A~2B":{"type":"string"}},"$ref":"#/$defs/A~2B"}"##)
            .expect_err("a local reference uses valid JSON Pointer escapes");
    assert_eq!(bad_pointer.reason(), LossReason::UnsupportedSource);
    assert_eq!(bad_pointer.path(), "$[\"$ref\"]");

    let ambiguous = json_schema::project(
        r#"{"oneOf":[{"type":"integer","minimum":-128,"maximum":127},{"type":"number"}]}"#,
    )
    .expect_err("overlapping logical branches refuse");
    assert_eq!(ambiguous.reason(), LossReason::AmbiguousUnion);
}

#[test]
fn a_json_value_selects_the_one_canonical_schema_branch() {
    let union = json_schema::project(
        r#"{"oneOf":[{"type":"integer","minimum":-128,"maximum":127},{"type":"string"}]}"#,
    )
    .expect("the branches are disjoint");
    let DataType::StructuralUnion(branches) = &union else {
        panic!("oneOf projects to a structural union");
    };
    let text_branch = branches
        .branches()
        .iter()
        .position(|branch| branch == &DataType::Text)
        .and_then(|index| u16::try_from(index).ok())
        .expect("the text branch has a wire-sized index");
    assert_eq!(
        union.narrow(Value::Text(String::from("ready")), None),
        Ok(Value::StructuralUnion {
            branch: text_branch,
            value: Box::new(Value::Text(String::from("ready"))),
        })
    );
}

#[test]
fn avro_supported_rows_project_to_the_exact_logical_type() {
    let rows = [
        (r#""null""#, DataType::Null),
        (r#""boolean""#, DataType::Bool),
        (
            r#""int""#,
            DataType::Integer(IntegerType::Signed(IntegerWidth::Bits32)),
        ),
        (
            r#"{"type":"fixed","name":"Digest","size":32}"#,
            DataType::Bytes(BinaryType::Fixed(
                ByteLength::new(32).expect("valid length"),
            )),
        ),
        (
            r#"{"type":"bytes","logicalType":"decimal","precision":18,"scale":4}"#,
            DataType::Decimal(DecimalType::new(18, 4).expect("valid decimal")),
        ),
        (
            r#"{"type":"int","logicalType":"date"}"#,
            DataType::Temporal(TemporalType::Date(DateUnit::Day)),
        ),
        (
            r#"{"type":"long","logicalType":"timestamp-micros"}"#,
            DataType::Temporal(TemporalType::Timestamp {
                unit: TimeUnit::Microsecond,
                zone: TimeZone::Utc,
            }),
        ),
        (
            r#"{"type":"fixed","name":"Window","size":12,"logicalType":"duration"}"#,
            DataType::Temporal(TemporalType::Interval(IntervalUnit::MonthDayMillisUnsigned)),
        ),
        (r#"{"type":"string","logicalType":"uuid"}"#, DataType::Uuid),
    ];
    for (document, expected) in rows {
        let projected = avro::project(document).expect(document);
        assert_eq!(projected.ty(), &expected, "{document}");
    }
}

#[test]
fn avro_names_and_field_order_resolve_before_canonical_identity() {
    let left = r#"{"type":"record","name":"Person","namespace":"app","fields":[{"name":"z","type":"long"},{"name":"a","type":"string"}]}"#;
    let right = r#"{"fields":[{"type":"string","name":"a"},{"type":"long","name":"z"}],"name":"other.Person","type":"record"}"#;
    let expected = structure([
        ("a", DataType::Text),
        (
            "z",
            DataType::Integer(IntegerType::Signed(IntegerWidth::Bits64)),
        ),
    ]);
    assert_eq!(avro::project(left).expect("left").ty(), &expected);
    assert_eq!(avro::project(right).expect("right").ty(), &expected);
}

#[test]
fn avro_union_indexes_are_explicit_and_outside_logical_identity() {
    let disjoint = avro::project(r#"["string","long"]"#).expect("disjoint union");
    assert_eq!(disjoint.unions().len(), 1);
    assert_eq!(
        disjoint.unions()[0].branches(),
        &[UnionSelection::Structural(1), UnionSelection::Structural(0)]
    );
    assert_eq!(
        disjoint.ty().narrow(Value::Text(String::from("ok")), None),
        Ok(Value::StructuralUnion {
            branch: 1,
            value: Box::new(Value::Text(String::from("ok"))),
        })
    );

    let tagged = avro::project(
        r#"[{"type":"record","name":"Right","fields":[{"name":"value","type":"long"}]},{"type":"record","name":"Left","fields":[{"name":"value","type":"long"}]}]"#,
    )
    .expect("same-shape names need tags");
    assert!(matches!(tagged.ty(), DataType::TaggedUnion(_)));
    assert_eq!(
        tagged.unions()[0].branches(),
        &[
            UnionSelection::Tagged(String::from("Right")),
            UnionSelection::Tagged(String::from("Left")),
        ]
    );
}

#[test]
fn avro_reverse_projection_round_trips_exact_types() {
    let tagged = DataType::TaggedUnion(
        TaggedUnionType::new(vec![
            TaggedCase::new(
                "A",
                StructType::new(vec![TypeField::new("value", DataType::Text)]).expect("payload"),
            )
            .expect("case"),
            TaggedCase::new(
                "B",
                StructType::new(vec![TypeField::new("value", DataType::Text)]).expect("payload"),
            )
            .expect("case"),
        ])
        .expect("union"),
    );
    let exact = [
        DataType::Text,
        DataType::Decimal(DecimalType::new(12, 2).expect("decimal")),
        DataType::array(DataType::Bool),
        structure([("id", DataType::LONG), ("name", DataType::Text)]),
        tagged,
    ];
    for ty in exact {
        let document = avro::reverse(&ty).expect("exact Avro projection");
        assert_eq!(avro::project(&document).expect(&document).ty(), &ty);
    }
}

#[test]
fn avro_refuses_recursive_and_oversized_logical_schemas() {
    let recursive = avro::project(
        r#"{"type":"record","name":"Node","fields":[{"name":"next","type":"Node"}]}"#,
    )
    .expect_err("recursive record refuses");
    assert_eq!(recursive.reason(), LossReason::RecursiveSchema);

    let decimal =
        avro::project(r#"{"type":"bytes","logicalType":"decimal","precision":39,"scale":2}"#)
            .expect_err("precision outside the decimal128 profile refuses");
    assert_eq!(decimal.reason(), LossReason::UnsupportedSource);
    assert_eq!(decimal.path(), "$");

    let fixed_decimal = avro::project(
        r#"{"type":"fixed","name":"Tiny","size":1,"logicalType":"decimal","precision":3,"scale":0}"#,
    )
    .expect_err("three decimal digits do not fit one signed byte");
    assert_eq!(fixed_decimal.reason(), LossReason::InvalidSchema);
    assert_eq!(fixed_decimal.path(), "$.precision");

    let forward = avro::project(
        r#"{"type":"record","name":"Outer","fields":[{"name":"first","type":"Later"},{"name":"definition","type":{"type":"record","name":"Later","fields":[]}}]}"#,
    )
    .expect_err("Avro names must be defined before their first use");
    assert_eq!(forward.reason(), LossReason::InvalidSchema);
    assert_eq!(forward.path(), "$.fields[0].type");

    let duplicate_name = avro::project(
        r#"{"type":"record","name":"Outer","namespace":"ns","fields":[{"name":"definition","type":{"type":"record","name":"A","fields":[]}},{"name":"bad","type":["A",{"type":"ns.A"}]}]}"#,
    )
    .expect_err("two spellings of one named type are one duplicate union branch");
    assert_eq!(duplicate_name.reason(), LossReason::InvalidSchema);
    assert_eq!(duplicate_name.path(), "$.fields[1].type[1]");

    for duplicate in [
        r#"[{"type":"array","items":"string"},{"type":"array","items":"long"}]"#,
        r#"[{"type":"long","logicalType":"timestamp-millis"},{"type":"long","logicalType":"timestamp-micros"}]"#,
    ] {
        let error = avro::project(duplicate)
            .expect_err("Avro union uniqueness follows the physical schema type");
        assert_eq!(error.reason(), LossReason::InvalidSchema, "{duplicate}");
        assert_eq!(error.path(), "$[1]", "{duplicate}");
    }
}

#[test]
fn arrow_supported_rows_project_to_the_exact_logical_type() {
    let rows = [
        (
            ArrowType::Integer(IntegerType::Unsigned(IntegerWidth::Bits32)),
            DataType::Integer(IntegerType::Unsigned(IntegerWidth::Bits32)),
        ),
        (
            ArrowType::Float(FloatType::Binary16),
            DataType::Float(FloatType::Binary16),
        ),
        (
            ArrowType::Decimal {
                width: ArrowDecimalWidth::Bits128,
                precision: 20,
                scale: -2,
            },
            DataType::Decimal(DecimalType::new(20, -2).expect("decimal")),
        ),
        (
            ArrowType::Binary(ArrowBinaryKind::LargeBinary),
            DataType::BYTES,
        ),
        (ArrowType::Text(ArrowTextKind::Utf8View), DataType::Text),
        (
            ArrowType::Timestamp {
                unit: TimeUnit::Nanosecond,
                timezone: Some(String::from("Europe/Paris")),
            },
            DataType::Temporal(TemporalType::Timestamp {
                unit: TimeUnit::Nanosecond,
                zone: TimeZone::named("Europe/Paris").expect("zone"),
            }),
        ),
        (
            ArrowType::Dictionary(Box::new(ArrowType::Text(ArrowTextKind::Utf8))),
            DataType::Text,
        ),
        (
            ArrowType::RunEndEncoded(Box::new(ArrowType::Bool)),
            DataType::Bool,
        ),
    ];
    for (source, expected) in rows {
        assert_eq!(arrow::project(&source).expect("supported").ty(), &expected);
    }
}

#[test]
fn arrow_fields_collections_and_union_codes_keep_all_logical_information() {
    let source = ArrowType::Struct(vec![
        ArrowField::nullable("name", ArrowType::Text(ArrowTextKind::LargeUtf8)),
        ArrowField::required(
            "labels",
            ArrowType::Map {
                key: Box::new(ArrowField::required(
                    "key",
                    ArrowType::Text(ArrowTextKind::Utf8),
                )),
                value: Box::new(ArrowField::nullable("value", ArrowType::Bool)),
            },
        ),
    ]);
    let expected = structure([
        ("name", optional(DataType::Text)),
        (
            "labels",
            DataType::Map(MapType::new(DataType::Text, optional(DataType::Bool)).expect("map")),
        ),
    ]);
    assert_eq!(arrow::project(&source).expect("struct").ty(), &expected);

    let union = ArrowType::Union {
        mode: ArrowUnionMode::Sparse,
        children: vec![
            ArrowUnionChild::nullable("text", 42, ArrowType::Text(ArrowTextKind::Utf8)),
            ArrowUnionChild::new(
                "count",
                -7,
                ArrowType::Integer(IntegerType::Signed(IntegerWidth::Bits64)),
            ),
        ],
    };
    let projected = arrow::project(&union).expect("union");
    assert!(matches!(projected.ty(), DataType::TaggedUnion(_)));
    assert_eq!(projected.unions()[0].mode(), ArrowUnionMode::Sparse);
    assert_eq!(projected.unions()[0].codes()[0].type_code(), 42);
    assert_eq!(projected.unions()[0].codes()[0].case(), "text");
    assert_eq!(projected.unions()[0].codes()[1].type_code(), -7);
    assert_eq!(projected.unions()[0].codes()[1].case(), "count");
    let value = Value::TaggedUnion(Box::new(TaggedUnionValue {
        case: String::from("text"),
        payload: vec![(
            String::from("value"),
            Value::StructuralUnion {
                branch: 1,
                value: Box::new(Value::Text(String::from("ready"))),
            },
        )],
    }));
    assert!(projected.ty().accepts(&value));
}

#[test]
fn arrow_reverse_projection_round_trips_exact_types() {
    let tagged = DataType::TaggedUnion(
        TaggedUnionType::new(vec![
            TaggedCase::new("no", StructType::new(vec![]).expect("payload")).expect("case"),
            TaggedCase::new(
                "yes",
                StructType::new(vec![TypeField::new("value", DataType::Text)]).expect("payload"),
            )
            .expect("case"),
        ])
        .expect("union"),
    );
    let exact = [
        DataType::Integer(IntegerType::Signed(IntegerWidth::Bits8)),
        DataType::Decimal(DecimalType::new(38, -4).expect("decimal")),
        DataType::Array(ArrayType::fixed(optional(DataType::Bool), 4).expect("array")),
        structure([("a", optional(DataType::Text)), ("z", DataType::BYTES)]),
        tagged,
    ];
    for ty in exact {
        let descriptor = arrow::reverse(&ty).expect("exact Arrow projection");
        assert_eq!(arrow::project(&descriptor).expect("round trip").ty(), &ty);
    }

    let too_many_cases = DataType::TaggedUnion(
        TaggedUnionType::new(
            (0..129)
                .map(|index| {
                    TaggedCase::new(
                        format!("Case{index:03}"),
                        StructType::new(vec![]).expect("empty payload"),
                    )
                    .expect("named case")
                })
                .collect(),
        )
        .expect("the Rastok union remains within its own member budget"),
    );
    let error = arrow::reverse(&too_many_cases)
        .expect_err("a flat Arrow union cannot address a 129th child");
    assert_eq!(error.reason(), LossReason::UnsupportedTarget);
    assert_eq!(error.path(), "$");
}

#[test]
fn arrow_refuses_extensions_invalid_keys_and_duplicate_union_codes() {
    let extension = arrow::project(&ArrowType::Extension(String::from("com.acme.uuid")))
        .expect_err("extensions need a profile");
    assert_eq!(extension.reason(), LossReason::UnsupportedSource);
    assert_eq!(extension.path(), "$");

    let scale = arrow::project(&ArrowType::Decimal {
        width: ArrowDecimalWidth::Bits128,
        precision: 38,
        scale: 39,
    })
    .expect_err("the V1 fixed-decimal profile bounds its signed scale");
    assert_eq!(scale.reason(), LossReason::UnsupportedSource);
    assert_eq!(scale.path(), "$");

    let key = arrow::project(&ArrowType::Map {
        key: Box::new(ArrowField::nullable("key", ArrowType::Bool)),
        value: Box::new(ArrowField::required(
            "value",
            ArrowType::Text(ArrowTextKind::Utf8),
        )),
    })
    .expect_err("nullable key refuses");
    assert_eq!(key.reason(), LossReason::InvalidMapKey);
    assert_eq!(key.path(), "$.key");

    let codes = arrow::project(&ArrowType::Union {
        mode: ArrowUnionMode::Dense,
        children: vec![
            ArrowUnionChild::new("a", 1, ArrowType::Bool),
            ArrowUnionChild::new("b", 1, ArrowType::Null),
        ],
    })
    .expect_err("duplicate type codes refuse");
    assert_eq!(codes.reason(), LossReason::InvalidSchema);
    assert_eq!(codes.path(), "$[1]");
}
