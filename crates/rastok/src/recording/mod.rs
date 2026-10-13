//! Persistent diagnostic captures, not a durable deployment Journal.
//! Readers reconstruct retained facts without physical Drivers or producers.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]

mod codec;
mod definitions;
mod replay;
#[cfg(feature = "hosted-ui")]
mod ui;
#[cfg(feature = "hosted-ui")]
pub use ui::serve_ui;
#[cfg(test)]
mod tests;

use crate::canonical_cbor::{self, Value as Cbor};
use crate::compiler::{Compilation, SourceDefinition};
use crate::deployment::{BootBindings, DeploymentAttestation};
use crate::journal::{Journal, MemJournal, Record};
use crate::plan::{self, GraphVersion, SemanticHash, ValidatedExecutionPlan};
use crate::runtime::execution::{Execution, ExecutionFault};
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::Path;

const MAGIC: &[u8; 4] = b"RKRC";
const VERSION: u64 = 4;
const CHECKSUM_DOMAIN: &[u8] = b"rastok.diagnostic-recording\0";
/// Upper bound on one diagnostic file, including its embedded plan package.
pub const MAX_RECORDING_BYTES: usize = 64 * 1024 * 1024;
const MAX_RECORDS: usize = 100_000;

/// A refusal of capture or read-only reconstruction.
#[derive(Debug)]
pub enum Error {
    /// Filesystem failure; a pre-existing recording is never replaced.
    Io(std::io::Error),
    /// Malformed or contradictory recording fields.
    Invalid(&'static str),
    /// An unimplemented recording capability, not a relaxed execution rule.
    Unsupported(&'static str),
    /// No qualified persistent personal-data retention capability exists yet.
    PersonalData,
    /// A format bound was exceeded.
    TooLarge,
    /// Independent canonical reader refusal.
    Read(canonical_cbor::DecodeError),
    /// Embedded plan validation refusal.
    Plan(plan::DecodeError),
    /// Retained journal acceptance refusal.
    Journal(crate::journal::Error),
    /// Retained-state reconstruction refusal.
    Recovery(ExecutionFault),
    /// Pure Contract projection could not bind.
    Contract(String),
    /// Physical debugger startup or projection failure.
    Ui(String),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "recording I/O: {error}"),
            Self::Invalid(reason) => write!(f, "invalid or incomplete recording: {reason}"),
            Self::Unsupported(reason) => write!(f, "recording capability unavailable: {reason}"),
            Self::PersonalData => f.write_str("recording @personal data requires qualified persistent retention/erasure; M1 supports only the run-scoped memory journal"),
            Self::TooLarge => write!(f, "recording exceeds the {} MiB / {MAX_RECORDS} record limit", MAX_RECORDING_BYTES / 1024 / 1024),
            Self::Read(error) => write!(f, "recording CBOR: {error}"),
            Self::Plan(error) => write!(f, "recording plan: {error}"),
            Self::Journal(error) => write!(f, "recording journal: {error}"),
            Self::Recovery(error) => write!(f, "recording reconstruction: {error}"),
            Self::Contract(error) => write!(f, "recording Contract projection: {error}"),
            Self::Ui(error) => write!(f, "recording UI: {error}"),
        }
    }
}
impl std::error::Error for Error {}

impl From<crate::journal::codec::Error> for Error {
    fn from(error: crate::journal::codec::Error) -> Self {
        match error {
            crate::journal::codec::Error::Invalid(reason) => Self::Invalid(reason),
            crate::journal::codec::Error::Read(error) => Self::Read(error),
            crate::journal::codec::Error::Deadlines(error) => {
                Self::Journal(crate::journal::Error::InvalidDeadlines(error))
            }
            crate::journal::codec::Error::CallTiming(error) => {
                Self::Journal(crate::journal::Error::InvalidCallRetention(error))
            }
        }
    }
}

/// Exact retained facts with their independently validated plan and source view.
pub struct Recording {
    plan: ValidatedExecutionPlan,
    hash: SemanticHash,
    definitions: Vec<SourceDefinition>,
    version: GraphVersion,
    records: Vec<Record>,
    final_time: i64,
    empty_deployment_header: bool,
    finished: bool,
    /// Prefix lengths and explicit observation instants; not causal timestamps.
    checkpoints: Vec<(usize, i64)>,
}
impl fmt::Debug for Recording {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Recording")
            .field("hash", &self.hash)
            .field("records", &self.records.len())
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

fn validate_capture(
    plan: &ValidatedExecutionPlan,
    header: Option<&DeploymentAttestation>,
) -> Result<(), Error> {
    let personal = crate::data_policy::contains_personal;
    if plan.resolved_ports().boundary().iter().any(|port| personal(port.annotations()))
        || plan.nodes.iter().enumerate().any(|(index, node)| {
            plan.resolved_ports().node(plan::NodeIndex::new(index as u32)).is_some_and(|ports| ports.iter().any(|port| personal(port.annotations())))
                || node.vars.iter().any(|var| personal(&var.annotations))
        })
        || plan.owners.iter().any(|owner| owner.ports.iter().any(|port| personal(&port.annotations)) || owner.vals.iter().any(|value| personal(&value.annotations)))
        || plan.handlers.iter().any(|handler| handler.dispatch.steps().any(|step| matches!(step, plan::DispatchStep::Let { annotations, .. } if personal(annotations))))
    { return Err(Error::PersonalData); }
    if header.is_some_and(|header| header != &DeploymentAttestation::default())
        || plan.nodes.iter().filter_map(|node| node.connector()).any(|connector| connector.attributes.iter().any(|(_, value)| matches!(value, plan::AttrSetting::Expr { expr, .. } if crate::expr::boot_reference(&expr.root).is_ok_and(|reference| reference.is_some()))))
    {
        return Err(Error::Unsupported("boot-dependent profiles need a reference-only recording header; no secret or environment value is copied implicitly"));
    }
    Ok(())
}

impl Recording {
    /// Copies one completed retained run after checking its persistence policy.
    ///
    /// # Errors
    /// An unsupported deployment policy, bound or invalid prefix.
    pub fn capture(
        compilation: &Compilation,
        journal: &MemJournal,
        final_time: i64,
    ) -> Result<Self, Error> {
        validate_capture(&compilation.plan, journal.deployment_attestation())?;
        if journal.records().len() > MAX_RECORDS {
            return Err(Error::TooLarge);
        }
        let value = Self {
            plan: compilation.plan.clone(),
            hash: compilation.hash,
            definitions: compilation.source_definitions.clone(),
            version: journal.graph_version(),
            records: journal.records().to_vec(),
            final_time,
            empty_deployment_header: journal.deployment_attestation().is_some(),
            finished: true,
            checkpoints: vec![(journal.records().len(), final_time)],
        };
        value.inspect(value.records.len())?;
        Ok(value)
    }

    /// Checked plan retained by this recording.
    #[must_use]
    pub fn plan(&self) -> &ValidatedExecutionPlan {
        &self.plan
    }
    /// Declared semantic identity of that package.
    #[must_use]
    pub fn hash(&self) -> SemanticHash {
        self.hash
    }
    /// Exact authored owner definitions, presentation only.
    #[must_use]
    pub fn definitions(&self) -> &[SourceDefinition] {
        &self.definitions
    }
    /// Retained records in accepting order.
    #[must_use]
    pub fn records(&self) -> &[Record] {
        &self.records
    }
    /// Whether the writer retained a normal end-of-run footer. A complete
    /// prefix without a footer remains inspectable, but is never called complete.
    #[must_use]
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// Reads one complete bounded file, checks integrity, then validates its facts.
    /// It opens no path mentioned by the embedded program.
    ///
    /// # Errors
    /// Read, integrity, plan or journal validation failure.
    pub fn read(path: &Path) -> Result<Self, Error> {
        // Refuse ordinary pipes/devices before opening them. The second check
        // validates the opened handle too; no program-owned path is followed.
        if !std::fs::metadata(path).map_err(Error::Io)?.is_file() {
            return Err(Error::Invalid("recording must be a regular file"));
        }
        let file = File::open(path).map_err(Error::Io)?;
        if !file.metadata().map_err(Error::Io)?.is_file() {
            return Err(Error::Invalid("recording must be a regular file"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_RECORDING_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(Error::Io)?;
        Self::decode(&bytes)
    }

    /// Encodes a diagnostic format; neither plan nor causal-record bytes change.
    ///
    /// # Errors
    /// The diagnostic capture exceeds its documented size bound.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let header = codec::list([
            Cbor::Uint(VERSION),
            Cbor::Bytes(plan::encode(&self.plan, &self.hash)),
            Cbor::Uint(self.version.0),
            Cbor::Bool(self.empty_deployment_header),
            definitions::encode(&self.definitions),
        ]);
        let mut bytes = MAGIC.to_vec();
        let mut seal = [0; 32];
        bytes.extend(frame(&header, &mut seal)?);
        let mut start = 0;
        for &(end, at) in &self.checkpoints {
            bytes.extend(frame(
                &codec::list([
                    Cbor::Uint(1),
                    codec::integer(at),
                    codec::list(self.records[start..end].iter().map(codec::encode)),
                ]),
                &mut seal,
            )?);
            start = end;
        }
        if self.finished {
            bytes.extend(frame(&footer(&self.records), &mut seal)?);
        }
        if bytes.len() > MAX_RECORDING_BYTES {
            return Err(Error::TooLarge);
        }
        Ok(bytes)
    }

    /// Strict independent reader. A checksum is corruption detection, not a signature.
    ///
    /// # Errors
    /// Unknown version, malformed values, missing facts or failed reconstruction.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_RECORDING_BYTES {
            return Err(Error::TooLarge);
        }
        let mut rest = bytes.strip_prefix(MAGIC).ok_or(Error::Invalid("magic"))?;
        let mut seal = [0; 32];
        let tree = read_frame(&mut rest, &mut seal)?;
        let [format, package, version, header, source] = codec::array(&tree)?;
        if codec::uint(format)? != VERSION {
            return Err(Error::Unsupported("recording format version"));
        }
        let (plan, hash) = plan::decode(codec::bytes(package)?).map_err(Error::Plan)?;
        let Cbor::Bool(empty_deployment_header) = header else {
            return Err(Error::Invalid("deployment header"));
        };
        validate_capture(&plan, None)?;
        let mut reader = codec::Decoder::default();
        let mut records = Vec::new();
        let mut final_time = None;
        let mut finished = false;
        let mut checkpoints = Vec::new();
        while !rest.is_empty() {
            let row = read_frame(&mut rest, &mut seal)?;
            match codec::items(&row)? {
                [Cbor::Uint(1), at, retained] if !finished => {
                    let at = codec::signed(at)?;
                    if final_time.is_some_and(|previous| at < previous) {
                        return Err(Error::Invalid("checkpoint time moved backwards"));
                    }
                    final_time = Some(at);
                    let rows = codec::items(retained)?;
                    if records.len().saturating_add(rows.len()) > MAX_RECORDS {
                        return Err(Error::TooLarge);
                    }
                    for row in rows {
                        let record = reader.decode(row)?;
                        if codec::encode(&record) != *row {
                            return Err(Error::Invalid("non-canonical or lossy record"));
                        }
                        records.push(record);
                    }
                    checkpoints.push((records.len(), at));
                }
                [Cbor::Uint(2), _, _] if !finished && final_time.is_some() => {
                    if row != footer(&records) {
                        return Err(Error::Invalid("record count or causal seal"));
                    }
                    finished = true;
                }
                _ => return Err(Error::Invalid("checkpoint or final footer")),
            }
        }
        let value = Self {
            plan,
            hash,
            version: GraphVersion(codec::uint(version)?),
            final_time: final_time.ok_or(Error::Invalid("no retained checkpoint"))?,
            empty_deployment_header: *empty_deployment_header,
            definitions: definitions::decode(source)?,
            records,
            finished,
            checkpoints,
        };
        if value.encode()? != bytes {
            return Err(Error::Invalid("non-canonical recording"));
        }
        value.inspect(value.records.len())?;
        Ok(value)
    }

    /// Reconstructs a read-only prefix. No handler, guest or physical Driver runs.
    /// Time is the containing checkpoint's observation instant, not an invented
    /// timestamp for the selected record's commit.
    ///
    /// # Errors
    /// A prefix outside the retained run or a failed journal/VM validation.
    pub fn inspect(&self, through: usize) -> Result<(MemJournal, Execution<'_>), Error> {
        let prefix = self
            .records
            .get(..through)
            .ok_or(Error::Invalid("prefix index"))?;
        let header = self
            .empty_deployment_header
            .then(DeploymentAttestation::default);
        let journal =
            MemJournal::from_records(self.version, header, prefix).map_err(Error::Journal)?;
        // Acceptance may canonicalize collections, but must never rewrite history.
        if prefix
            .iter()
            .zip(journal.records())
            .any(|(before, after)| codec::encode(before) != codec::encode(after))
        {
            return Err(Error::Invalid("journal acceptance changed a retained fact"));
        }
        let now = if through == self.records.len() {
            self.final_time
        } else {
            self.checkpoints
                .iter()
                .find(|(end, _)| *end >= through)
                .map(|(_, at)| *at)
                .ok_or(Error::Invalid("missing prefix observation"))?
        };
        let empty = BootBindings::empty();
        let mut vm = Execution::boot_program(
            &self.plan,
            self.hash,
            self.version,
            now,
            self.empty_deployment_header.then_some(&empty),
        );
        let mut contracts = replay::Contracts::bind(&self.plan)?;
        vm.recover_with_bindings(&journal, &mut contracts)
            .map_err(Error::Recovery)?;
        Ok((journal, vm))
    }
}

fn checksum(previous: &[u8; 32], body: &[u8]) -> [u8; 32] {
    let mut digest = blake3::Hasher::new();
    digest.update(CHECKSUM_DOMAIN);
    digest.update(previous);
    digest.update(body);
    *digest.finalize().as_bytes()
}

fn frame(value: &Cbor, seal: &mut [u8; 32]) -> Result<Vec<u8>, Error> {
    let body = canonical_cbor::encode(value);
    if body.len() > MAX_RECORDING_BYTES {
        return Err(Error::TooLarge);
    }
    let length = u32::try_from(body.len()).map_err(|_| Error::TooLarge)?;
    *seal = checksum(seal, &body);
    let mut bytes = length.to_be_bytes().to_vec();
    bytes.extend_from_slice(seal);
    bytes.extend(body);
    Ok(bytes)
}
fn read_frame(bytes: &mut &[u8], seal: &mut [u8; 32]) -> Result<Cbor, Error> {
    let (length, rest) = bytes
        .split_at_checked(4)
        .ok_or(Error::Invalid("truncated frame length"))?;
    let length = u32::from_be_bytes(
        length
            .try_into()
            .map_err(|_| Error::Invalid("frame length"))?,
    ) as usize;
    let (expected, rest) = rest
        .split_at_checked(32)
        .ok_or(Error::Invalid("truncated frame checksum"))?;
    let (body, rest) = rest
        .split_at_checked(length)
        .ok_or(Error::Invalid("truncated frame body"))?;
    let actual = checksum(seal, body);
    if expected != actual {
        return Err(Error::Invalid("checksum"));
    }
    let (tree, used) = canonical_cbor::decode(body).map_err(Error::Read)?;
    if used != body.len() {
        return Err(Error::Invalid("trailing bytes in frame"));
    }
    *seal = actual;
    *bytes = rest;
    Ok(tree)
}
fn footer(records: &[Record]) -> Cbor {
    codec::list([
        Cbor::Uint(2),
        Cbor::Uint(records.len() as u64),
        codec::word(crate::golden::project(records).seal_hex()),
    ])
}

/// An append-only diagnostic destination. Complete checkpoints survive a
/// missing footer, but no write-before-effect or crash-recovery guarantee is made.
#[derive(Debug)]
pub struct Recorder {
    file: File,
    version: GraphVersion,
    hash: SemanticHash,
    written: usize,
    size: usize,
    seal: [u8; 32],
    checkpointed: bool,
    last_time: Option<i64>,
}
impl Recorder {
    /// Validates persistence policy and reserves a new file before execution.
    /// Existing paths and symlinks are never overwritten.
    ///
    /// # Errors
    /// Unsupported personal-data/deployment capability or file creation failure.
    pub fn create(
        path: &Path,
        compilation: &Compilation,
        version: GraphVersion,
        boot: Option<&BootBindings>,
    ) -> Result<Self, Error> {
        validate_capture(&compilation.plan, boot.map(BootBindings::attestation))?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(Error::Io)?;
        file.write_all(MAGIC).map_err(Error::Io)?;
        let mut recorder = Self {
            file,
            version,
            hash: compilation.hash,
            written: 0,
            size: MAGIC.len(),
            seal: [0; 32],
            checkpointed: false,
            last_time: None,
        };
        recorder.append(&codec::list([
            Cbor::Uint(VERSION),
            Cbor::Bytes(plan::encode(&compilation.plan, &compilation.hash)),
            Cbor::Uint(version.0),
            Cbor::Bool(boot.is_some()),
            definitions::encode(&compilation.source_definitions),
        ]))?;
        Ok(recorder)
    }

    fn append(&mut self, value: &Cbor) -> Result<(), Error> {
        let bytes = frame(value, &mut self.seal)?;
        if self.size.saturating_add(bytes.len()) > MAX_RECORDING_BYTES {
            return Err(Error::TooLarge);
        }
        self.file.write_all(&bytes).map_err(Error::Io)?;
        self.size += bytes.len();
        Ok(())
    }

    /// Appends new committed records at an explicit observation instant. This
    /// instant does not claim to date every record's commit.
    ///
    /// # Errors
    /// Size bound, non-monotonic prefix or storage failure.
    pub fn checkpoint(&mut self, journal: &MemJournal, now: i64) -> Result<(), Error> {
        if journal.graph_version() != self.version {
            return Err(Error::Invalid("recording deployment version changed"));
        }
        if self.last_time.is_some_and(|previous| now < previous) {
            return Err(Error::Invalid("checkpoint time moved backwards"));
        }
        if journal.records().len() > MAX_RECORDS {
            return Err(Error::TooLarge);
        }
        let suffix = journal
            .records()
            .get(self.written..)
            .ok_or(Error::Invalid("journal prefix shrank"))?;
        if suffix.is_empty() && self.checkpointed {
            return Ok(());
        }
        self.append(&codec::list([
            Cbor::Uint(1),
            codec::integer(now),
            codec::list(suffix.iter().map(codec::encode)),
        ]))?;
        self.written = journal.records().len();
        self.checkpointed = true;
        self.last_time = Some(now);
        Ok(())
    }

    /// Finalizes only after the complete prefix validates. No partially encoded
    /// capture is presented as a successful recording.
    ///
    /// # Errors
    /// Unsupported prefix, size bound, write or synchronization failure.
    pub fn finish(
        mut self,
        compilation: &Compilation,
        journal: &MemJournal,
        now: i64,
    ) -> Result<(), Error> {
        if compilation.hash != self.hash {
            return Err(Error::Invalid("recording plan changed"));
        }
        Recording::capture(compilation, journal, now)?;
        self.checkpoint(journal, now)?;
        self.append(&codec::list([
            Cbor::Uint(1),
            codec::integer(now),
            codec::list([]),
        ]))?;
        self.append(&footer(journal.records()))?;
        self.file.sync_all().map_err(Error::Io)?;
        Ok(())
    }
}
