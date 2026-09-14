use super::{Error, codec};
use crate::canonical_cbor::Value as Cbor;
use crate::compiler::{SourceArm, SourceDefinition};
use crate::diagnostics::SourceId;
use crate::node::Kind;
use crate::plan::OwnerPath;

pub(super) fn encode(definitions: &[SourceDefinition]) -> Cbor {
    codec::list(definitions.iter().map(|definition| {
        codec::list([
            codec::word(&definition.owner),
            codec::word(match definition.kind {
                Kind::Graph => "graph",
                Kind::Operator => "operator",
                Kind::Connector => "connector",
            }),
            Cbor::Uint(u64::from(definition.source.index())),
            Cbor::Uint(definition.first_line as u64),
            Cbor::Uint(definition.last_line as u64),
            codec::word(&definition.code),
            codec::list(definition.decisions.iter().map(|(trigger, arms)| {
                codec::list([codec::word(trigger), codec::list(arms.iter().map(arm))])
            })),
        ])
    }))
}
fn arm(value: &SourceArm) -> Cbor {
    codec::list([
        codec::list(value.cases.iter().map(codec::word)),
        value.predicate.as_ref().map_or(Cbor::Null, codec::word),
        Cbor::Bool(value.fallback),
        Cbor::Uint(value.lines.0 as u64),
        Cbor::Uint(value.lines.1 as u64),
        codec::list(value.children.iter().map(arm)),
    ])
}
fn index(value: &Cbor) -> Result<usize, Error> {
    usize::try_from(codec::uint(value)?).map_err(|_| Error::Invalid("source position"))
}
pub(super) fn decode(value: &Cbor) -> Result<Vec<SourceDefinition>, Error> {
    codec::items(value)?
        .iter()
        .map(|value| {
            let [owner, kind, source, first, last, code, decisions] = codec::array(value)?;
            let owner = codec::text(owner)?;
            Ok(SourceDefinition {
                owner: OwnerPath::new(owner.split('.').filter(|segment| !segment.is_empty()))
                    .map_err(|_| Error::Invalid("definition owner"))?,
                kind: match codec::text(kind)? {
                    "graph" => Kind::Graph,
                    "operator" => Kind::Operator,
                    "connector" => Kind::Connector,
                    _ => return Err(Error::Invalid("definition kind")),
                },
                source: SourceId::new(
                    u32::try_from(codec::uint(source)?).map_err(|_| Error::Invalid("source id"))?,
                ),
                first_line: index(first)?,
                last_line: index(last)?,
                code: codec::text(code)?.to_owned(),
                decisions: codec::items(decisions)?
                    .iter()
                    .map(|value| {
                        let [trigger, arms] = codec::array(value)?;
                        Ok((
                            codec::text(trigger)?.to_owned(),
                            codec::items(arms)?
                                .iter()
                                .map(read_arm)
                                .collect::<Result<_, _>>()?,
                        ))
                    })
                    .collect::<Result<_, Error>>()?,
            })
        })
        .collect()
}
fn read_arm(value: &Cbor) -> Result<SourceArm, Error> {
    let [cases, predicate, fallback, first, last, children] = codec::array(value)?;
    let Cbor::Bool(fallback) = fallback else {
        return Err(Error::Invalid("source fallback"));
    };
    Ok(SourceArm {
        cases: codec::items(cases)?
            .iter()
            .map(|value| codec::text(value).map(str::to_owned))
            .collect::<Result<_, _>>()?,
        predicate: if matches!(predicate, Cbor::Null) {
            None
        } else {
            Some(codec::text(predicate)?.to_owned())
        },
        fallback: *fallback,
        lines: (index(first)?, index(last)?),
        children: codec::items(children)?
            .iter()
            .map(read_arm)
            .collect::<Result<_, _>>()?,
    })
}
