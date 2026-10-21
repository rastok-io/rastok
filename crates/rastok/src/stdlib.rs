//! Standard tagged frontends, annotations and media-type completion names.
//!
//! This module supplies the compiler's frontend and annotation declarations
//! and the media-type names offered by tooling, as specified in [`stdlib.md`].
//! Standard Connector definitions live separately in [`crate::connector::stdlib`]
//! and are shared with the independent package validator.
//!
//! [`stdlib.md`]: https://github.com/rastok-io/rastok/blob/main/docs/spec/stdlib.md

use crate::contract::{FrontendContract, FrontendFraming, FrontendReader};

/// Hosted declarations; runtime interpretation is retained in the resolved plan.
pub(crate) const ANNOTATION_DEFINITIONS: &str = include_str!("stdlib/annotations.rkg");

/// One completion candidate for the open `media_type` logical scalar.
///
/// This is deliberately a catalogue, not an enum: registered and vendor
/// media types remain valid when they parse canonically even when they are
/// absent here. Tooling may offer these stable names without turning today's
/// list into tomorrow's compatibility constraint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaTypeCatalogEntry {
    /// Completion-friendly Rastok spelling.
    pub name: &'static str,
    /// Canonical concrete media type.
    pub value: &'static str,
}

/// Common Internet media types offered by standard tooling.
pub const MEDIA_TYPE_CATALOG: &[MediaTypeCatalogEntry] = &[
    MediaTypeCatalogEntry {
        name: "application_cbor",
        value: "application/cbor",
    },
    MediaTypeCatalogEntry {
        name: "application_form_urlencoded",
        value: "application/x-www-form-urlencoded",
    },
    MediaTypeCatalogEntry {
        name: "application_json",
        value: "application/json",
    },
    MediaTypeCatalogEntry {
        name: "application_ndjson",
        value: "application/x-ndjson",
    },
    MediaTypeCatalogEntry {
        name: "application_octet_stream",
        value: "application/octet-stream",
    },
    MediaTypeCatalogEntry {
        name: "application_pdf",
        value: "application/pdf",
    },
    MediaTypeCatalogEntry {
        name: "application_problem_json",
        value: "application/problem+json",
    },
    MediaTypeCatalogEntry {
        name: "application_protobuf",
        value: "application/protobuf",
    },
    MediaTypeCatalogEntry {
        name: "application_xml",
        value: "application/xml",
    },
    MediaTypeCatalogEntry {
        name: "image_jpeg",
        value: "image/jpeg",
    },
    MediaTypeCatalogEntry {
        name: "image_png",
        value: "image/png",
    },
    MediaTypeCatalogEntry {
        name: "multipart_form_data",
        value: "multipart/form-data",
    },
    MediaTypeCatalogEntry {
        name: "text_csv",
        value: "text/csv",
    },
    MediaTypeCatalogEntry {
        name: "text_html",
        value: "text/html",
    },
    MediaTypeCatalogEntry {
        name: "text_plain_utf8",
        value: "text/plain; charset=utf-8",
    },
];

pub(crate) const M0_FRONTENDS: &[FrontendContract] = &[
    FrontendContract {
        name: "json",
        reader: FrontendReader::Json,
    },
    FrontendContract {
        name: "text",
        reader: FrontendReader::Text,
    },
    FrontendContract {
        name: "base64",
        reader: FrontendReader::Base64,
    },
    FrontendContract {
        name: "jq",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the jq guest frontend",
        },
    },
    FrontendContract {
        name: "sql",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the SQL guest frontend",
        },
    },
    FrontendContract {
        name: "bash",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the bash guest frontend",
        },
    },
    FrontendContract {
        name: "python",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Python guest frontend",
        },
    },
    FrontendContract {
        name: "rust",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Rust guest frontend",
        },
    },
    FrontendContract {
        name: "avro",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Avro data frontend",
        },
    },
    FrontendContract {
        name: "protobuf",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Protobuf data frontend",
        },
    },
    FrontendContract {
        name: "parquet",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Parquet data frontend",
        },
    },
    FrontendContract {
        name: "msgpack",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the MessagePack data frontend",
        },
    },
    FrontendContract {
        name: "csv",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the CSV data frontend",
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connector::contract::ContractCatalog;

    #[test]
    fn the_media_type_catalog_is_open_but_every_completion_is_canonical() {
        let mut names = std::collections::BTreeSet::new();
        let mut values = std::collections::BTreeSet::new();
        for entry in MEDIA_TYPE_CATALOG {
            assert!(names.insert(entry.name), "duplicate catalogue name");
            assert!(values.insert(entry.value), "duplicate catalogue value");
            assert_eq!(
                crate::datatype::canonical_media_type(entry.value),
                Ok(entry.value.to_string())
            );
        }
        assert_eq!(
            crate::datatype::canonical_media_type("application/vnd.example+json"),
            Ok(String::from("application/vnd.example+json")),
            "a valid value outside the completion catalogue remains admitted"
        );
    }

    #[test]
    fn frontends_share_one_contract_registry() {
        let registry = ContractCatalog::try_m0().expect("the standard catalogue holds");
        assert_eq!(
            registry.frontend("json").map(|frontend| frontend.reader),
            Some(FrontendReader::Json)
        );
        assert!(matches!(
            registry.frontend("jq").map(|frontend| frontend.reader),
            Some(FrontendReader::Deferred { .. })
        ));
        assert_eq!(
            registry
                .frontend("text")
                .map(|frontend| frontend.reader.result_type()),
            Some(Some(crate::datatype::DataType::Text))
        );
        assert_eq!(registry.frontend("starlark"), None);
    }
}
