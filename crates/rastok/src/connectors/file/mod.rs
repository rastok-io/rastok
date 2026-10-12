// The Driver layer IS the OS boundary: filesystem observations, threads, and
// cancellation races are its business — the semantic core stays pure.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
//! The real one-file Driver behind `std/file@3`.
//!
//! Request preparation is pure: it validates the logical path and seals one
//! complete operation descriptor. Only [`OutboundConnectorDriver::start`],
//! called after the EGRESS open is journaled, hands that descriptor to a
//! worker that may observe or mutate the filesystem.

use crate::boundary::{
    CallConnectorDriver, CallOpen, CallStart, Completion, FileExit, OutboundConnectorDriver,
    PreparedRequest, RequestPreparation, Response, SchedulerCompletion,
};
use crate::envelope::{Data, Encoding, Envelope, Value};
use crate::plan::{AttrSetting, NodeExecution};
use std::collections::BTreeMap;
use std::fs::{File as FsFile, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

const REQUEST_MAGIC: &[u8; 4] = b"RF1\0";
const READ_TAG: u8 = 1;
const REPLACE_TAG: u8 = 2;
const IO_CHUNK_BYTES: usize = 8 * 1024;
const STAGING_ATTEMPTS: u8 = 16;

#[derive(Debug, Clone)]
enum FileMode {
    Read { max_read: u64 },
    Replace,
}

#[derive(Debug, Clone)]
struct FileBinding {
    mode: FileMode,
    path: Option<String>,
}

impl FileBinding {
    fn try_from(spec: &crate::plan::ConnectorSpec) -> Result<Self, FileSettingsError> {
        use FileSettingsError as E;

        if spec.contract.as_str() != crate::connector::FILE_CONNECTOR_ID {
            return Err(E::NotFile(spec.contract.clone()));
        }
        let mode = match spec.attribute("mode") {
            Some(AttrSetting::Word(word)) if word == "read" => {
                let max_read = match spec.attribute("max_read") {
                    Some(AttrSetting::Value(Value::Text(text))) => {
                        crate::contract::byte_size(text).ok_or_else(|| E::NotASize(text.clone()))?
                    }
                    _ => return Err(E::Unresolved("max_read")),
                };
                FileMode::Read { max_read }
            }
            Some(AttrSetting::Word(word)) if word == "replace" => FileMode::Replace,
            Some(AttrSetting::Word(word)) => return Err(E::UnknownMode(word.clone())),
            _ => return Err(E::Unresolved("mode")),
        };
        let path = match spec.attribute("path") {
            Some(AttrSetting::Value(Value::Text(path))) => Some(path.clone()),
            Some(AttrSetting::Expr { .. }) => None,
            Some(_) => return Err(E::PathNotText),
            None => return Err(E::Unresolved("path")),
        };
        if let Some(setting) = spec.attribute("timeout")
            && !matches!(
                setting,
                AttrSetting::Value(Value::Duration(_)) | AttrSetting::Expr { .. }
            )
        {
            return Err(E::TimeoutNotDuration);
        }
        Ok(Self { mode, path })
    }

    fn path<'a>(&'a self, settings: &'a BTreeMap<String, Value>) -> Result<&'a str, FileExit> {
        if let Some(path) = &self.path {
            return Ok(path);
        }
        match settings.get("path") {
            Some(Value::Text(path)) => Ok(path),
            _ => Err(FileExit::InvalidPath),
        }
    }

    fn admits(&self, operation: &FileOperation, settings: &BTreeMap<String, Value>) -> bool {
        if self.path(settings).ok() != Some(operation.path()) {
            return false;
        }
        matches!(
            (&self.mode, operation),
            (
                FileMode::Read { max_read: expected },
                FileOperation::Read { max_read: actual, .. }
            ) if expected == actual
        ) || matches!(
            (&self.mode, operation),
            (FileMode::Replace, FileOperation::Replace { .. })
        )
    }
}

/// Pure, plan-bound half of the File protocol, shared by the real Driver and
/// the Lab's scripted Driver. It validates settings and seals operation bytes,
/// but holds no physical root and cannot observe the filesystem.
#[derive(Debug)]
pub(crate) struct FileRequestBindings {
    bindings: BTreeMap<crate::node::Path, FileBinding>,
}

impl FileRequestBindings {
    pub(crate) fn try_bind(
        plan: &crate::plan::ValidatedExecutionPlan,
        nodes: &[crate::node::Path],
    ) -> Result<Self, FileBindError> {
        let mut bindings = BTreeMap::new();
        for path in nodes {
            let Some(node) = plan.plan().nodes.iter().find(|node| &node.path == path) else {
                return Err(FileBindError {
                    node: path.clone(),
                    reason: FileSettingsError::Unresolved("connector binding path"),
                });
            };
            let NodeExecution::Connector(spec) = &node.execution else {
                return Err(FileBindError {
                    node: path.clone(),
                    reason: FileSettingsError::Unresolved("connector binding kind"),
                });
            };
            let binding = FileBinding::try_from(spec).map_err(|reason| FileBindError {
                node: path.clone(),
                reason,
            })?;
            bindings.insert(path.clone(), binding);
        }
        Ok(Self { bindings })
    }

    fn get(&self, connector: &crate::node::Path) -> Option<&FileBinding> {
        self.bindings.get(connector)
    }

    fn prepare(
        &self,
        connector: &crate::node::Path,
        data: &Data,
        settings: &BTreeMap<String, Value>,
    ) -> Result<PreparedRequest, FileExit> {
        let binding = self.get(connector).ok_or(FileExit::IoFailed)?;
        let path = validate_logical_path(binding.path(settings)?)?;
        let operation = match binding.mode {
            FileMode::Read { max_read } => FileOperation::Read { path, max_read },
            FileMode::Replace => {
                let bytes = crate::boundary::bytes_payload(data).map_err(|_| FileExit::IoFailed)?;
                return FileOperation::encode_replace(&path, bytes);
            }
        };
        operation.encode()
    }

    pub(crate) fn prepare_request(
        &self,
        connector: &crate::node::Path,
        data: &Data,
        settings: &BTreeMap<String, Value>,
    ) -> RequestPreparation {
        match self.prepare(connector, data, settings) {
            Ok(prepared) => RequestPreparation::Prepared(prepared),
            Err(exit) => RequestPreparation::Rejected(Completion::file_exit(exit)),
        }
    }
}

impl super::ScriptedConnectorProtocol for FileRequestBindings {
    fn prepare_request(
        &self,
        connector: &crate::node::Path,
        data: &Data,
        settings: &BTreeMap<String, Value>,
    ) -> RequestPreparation {
        self.prepare_request(connector, data, settings)
    }

    fn scheduler_completion(&self, completion: SchedulerCompletion) -> Completion {
        crate::boundary::file_scheduler_completion(completion)
    }
}

pub(super) fn bind_scripted_protocol(
    plan: &crate::plan::ValidatedExecutionPlan,
    nodes: &[crate::node::Path],
    _boot: &crate::deployment::BootBindings,
) -> Result<Box<dyn super::ScriptedConnectorProtocol>, super::DriverFactoryError> {
    FileRequestBindings::try_bind(plan, nodes)
        .map(|bindings| Box::new(bindings) as Box<dyn super::ScriptedConnectorProtocol>)
        .map_err(|error| super::DriverFactoryError::new(error.to_string()))
}

/// A violated `File` binding invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileSettingsError {
    /// The spec belongs to another Connector contract.
    NotFile(crate::connector::ContractId),
    /// A required effective setting did not travel in the plan.
    Unresolved(&'static str),
    /// The selected profile has no Driver in this build.
    UnknownMode(String),
    /// The path row is not text or a checked expression producing text.
    PathNotText,
    /// `max_read` is not a declared byte size.
    NotASize(String),
    /// `timeout` is not a duration.
    TimeoutNotDuration,
}

impl core::fmt::Display for FileSettingsError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotFile(contract) => write!(
                formatter,
                "the plan hands a `{contract}` connector to the File Driver"
            ),
            Self::Unresolved(name) => write!(
                formatter,
                "the plan violates the File contract: `{name}` is unresolved"
            ),
            Self::UnknownMode(mode) => {
                write!(
                    formatter,
                    "the File `{mode}` mode has no Driver in this build"
                )
            }
            Self::PathNotText => formatter.write_str("the File path attribute is not text"),
            Self::NotASize(text) => write!(formatter, "max_read `{text}` is not a size"),
            Self::TimeoutNotDuration => {
                formatter.write_str("the File timeout attribute is not a duration")
            }
        }
    }
}

impl std::error::Error for FileSettingsError {}

/// Why one selected File Node cannot bind before execution begins.
#[derive(Debug)]
pub struct FileBindError {
    /// Refusing Node.
    pub node: crate::node::Path,
    reason: FileSettingsError,
}

impl FileBindError {
    /// Structured binding reason.
    #[must_use]
    pub fn reason(&self) -> &FileSettingsError {
        &self.reason
    }
}

impl core::fmt::Display for FileBindError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "`{}` cannot bind: {}", self.node, self.reason)
    }
}

impl std::error::Error for FileBindError {}

/// Statically linked factory for `std/file@3`.
///
/// `root` is physical host configuration. It never enters the plan, request
/// digest, or causal identity, and construction does not inspect it.
#[derive(Debug)]
pub struct FileFactory {
    id: crate::connector::ContractId,
    root: PathBuf,
}

impl FileFactory {
    /// Constructs one File factory over an explicit physical resolution root.
    ///
    /// # Errors
    ///
    /// The built-in stable id ceased to be canonical.
    pub fn try_new(root: impl Into<PathBuf>) -> Result<Self, crate::connector::ConnectorIdError> {
        Ok(Self {
            id: crate::connector::ContractId::parse(crate::connector::FILE_CONNECTOR_ID)?,
            root: root.into(),
        })
    }
}

impl super::ConnectorDriverFactory for FileFactory {
    fn connector_id(&self) -> &crate::connector::ContractId {
        &self.id
    }

    fn roles(&self) -> crate::connector::ConnectorRoles {
        crate::connector::ConnectorRoles::Outbound
    }

    fn bind<'plan>(
        &self,
        plan: &'plan crate::plan::ValidatedExecutionPlan,
        nodes: &[crate::node::Path],
    ) -> Result<Box<dyn super::BoundConnectorDriver + 'plan>, super::DriverFactoryError> {
        File::try_bind(plan, nodes, self.root.clone())
            .map(|driver| Box::new(driver) as Box<dyn super::BoundConnectorDriver + 'plan>)
            .map_err(|error| super::DriverFactoryError::new(error.to_string()))
    }
}

#[derive(Debug)]
struct WorkerResult {
    call_id: crate::boundary::CallId,
    completion: Option<Completion>,
}

/// Real one-file outbound Driver.
///
/// Every call owns a cancellation flag and one detached worker. A worker
/// publishes no result when cancellation wins before an effect commits; if a
/// read or rename wins the race, its possibly-late completion remains visible
/// to the generic close-once dispatcher.
#[derive(Debug)]
pub struct File<'plan> {
    _plan: &'plan crate::plan::ValidatedExecutionPlan,
    root: PathBuf,
    bindings: FileRequestBindings,
    runs: BTreeMap<crate::boundary::CallId, Arc<AtomicBool>>,
    sender: mpsc::Sender<WorkerResult>,
    receiver: mpsc::Receiver<WorkerResult>,
    answers: Vec<Response>,
}

impl Drop for File<'_> {
    fn drop(&mut self) {
        for cancelled in self.runs.values() {
            cancelled.store(true, Ordering::Release);
        }
    }
}

impl super::BoundConnectorDriver for File<'_> {
    fn inbound(&mut self) -> Option<&mut dyn crate::boundary::InboundConnectorDriver> {
        None
    }

    fn outbound(&mut self) -> Option<&mut dyn OutboundConnectorDriver> {
        Some(self)
    }
}

impl<'plan> File<'plan> {
    fn try_bind(
        plan: &'plan crate::plan::ValidatedExecutionPlan,
        nodes: &[crate::node::Path],
        root: PathBuf,
    ) -> Result<Self, FileBindError> {
        let bindings = FileRequestBindings::try_bind(plan, nodes)?;
        let (sender, receiver) = mpsc::channel();
        Ok(Self {
            _plan: plan,
            root,
            bindings,
            runs: BTreeMap::new(),
            sender,
            receiver,
            answers: Vec::new(),
        })
    }
}

impl CallConnectorDriver for File<'_> {
    fn prepare_request(
        &mut self,
        connector: &crate::node::Path,
        data: &Data,
        resolved_settings: &BTreeMap<String, Value>,
    ) -> RequestPreparation {
        self.bindings
            .prepare_request(connector, data, resolved_settings)
    }

    fn scheduler_completion(
        &mut self,
        _connector: &crate::node::Path,
        completion: SchedulerCompletion,
    ) -> Completion {
        crate::boundary::file_scheduler_completion(completion)
    }

    fn cancel(&mut self, call_id: &crate::boundary::CallId) {
        if let Some(cancelled) = self.runs.get(call_id) {
            cancelled.store(true, Ordering::Release);
        }
    }

    fn start(&mut self, intent: &CallOpen, _request: &Envelope) -> CallStart {
        let Some(binding) = self.bindings.get(&intent.connector_path) else {
            self.answers.push(Response {
                call_id: intent.call_id,
                completion: Completion::file_exit(FileExit::IoFailed),
            });
            return CallStart::Started;
        };
        let operation = match FileOperation::decode(intent.prepared.bytes()) {
            Some(operation) => operation,
            None => {
                self.answers.push(Response {
                    call_id: intent.call_id,
                    completion: Completion::file_exit(FileExit::IoFailed),
                });
                return CallStart::Started;
            }
        };
        if validate_logical_path(operation.path()).is_err() {
            self.answers.push(Response {
                call_id: intent.call_id,
                completion: Completion::file_exit(FileExit::InvalidPath),
            });
            return CallStart::Started;
        }
        if !binding.admits(&operation, &intent.resolved_settings) {
            self.answers.push(Response {
                call_id: intent.call_id,
                completion: Completion::file_exit(FileExit::IoFailed),
            });
            return CallStart::Started;
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let root = self.root.clone();
        let sender = self.sender.clone();
        let call_id = intent.call_id;
        let spawn = std::thread::Builder::new()
            .name(String::from("rastok-file"))
            .spawn(move || {
                let completion = execute(&root, operation, &worker_cancelled, call_id);
                let _ = sender.send(WorkerResult {
                    call_id,
                    completion,
                });
            });
        if spawn.is_err() {
            self.answers.push(Response {
                call_id,
                completion: Completion::file_exit(FileExit::IoFailed),
            });
        } else {
            self.runs.insert(call_id, cancelled);
        }
        CallStart::Started
    }

    fn events(&mut self) -> Vec<Response> {
        let mut answers = std::mem::take(&mut self.answers);
        for result in self.receiver.try_iter() {
            self.runs.remove(&result.call_id);
            if let Some(completion) = result.completion {
                answers.push(Response {
                    call_id: result.call_id,
                    completion,
                });
            }
        }
        // A single poll may observe several independent worker completions.
        // Their OS scheduling order is not semantic; correlation identity is
        // the stable tie-break before the dispatcher journals them.
        answers.sort_by_key(|response| response.call_id);
        answers
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FileOperation {
    Read { path: String, max_read: u64 },
    Replace { path: String, bytes: Vec<u8> },
}

impl FileOperation {
    fn path(&self) -> &str {
        match self {
            Self::Read { path, .. } | Self::Replace { path, .. } => path,
        }
    }

    /// Stable File-driver request framing. These bytes are request-digest
    /// material: magic, mode, big-endian path length, UTF-8 path, then either
    /// the read bound or a big-endian payload length and exact payload.
    fn encode(&self) -> Result<PreparedRequest, FileExit> {
        match self {
            Self::Read { path, max_read } => encode_operation(READ_TAG, path, 8, |encoded| {
                encoded.extend_from_slice(&max_read.to_be_bytes());
                Ok(())
            }),
            Self::Replace { path, bytes } => Self::encode_replace(path, bytes),
        }
    }

    fn encode_replace(path: &str, bytes: &[u8]) -> Result<PreparedRequest, FileExit> {
        encode_operation(
            REPLACE_TAG,
            path,
            8usize.saturating_add(bytes.len()),
            |encoded| {
                let length = u64::try_from(bytes.len()).map_err(|_| FileExit::IoFailed)?;
                encoded.extend_from_slice(&length.to_be_bytes());
                encoded.extend_from_slice(bytes);
                Ok(())
            },
        )
    }

    fn decode(encoded: &[u8]) -> Option<Self> {
        let (&tag, rest) = encoded.strip_prefix(REQUEST_MAGIC)?.split_first()?;
        let (path_len, rest) = take_u32(rest)?;
        let path_len = usize::try_from(path_len).ok()?;
        let (path, rest) = take(rest, path_len)?;
        let path = String::from_utf8(path.to_vec()).ok()?;
        match tag {
            READ_TAG => {
                let (max_read, rest) = take_u64(rest)?;
                rest.is_empty().then_some(Self::Read { path, max_read })
            }
            REPLACE_TAG => {
                let (length, rest) = take_u64(rest)?;
                let length = usize::try_from(length).ok()?;
                (rest.len() == length).then(|| Self::Replace {
                    path,
                    bytes: rest.to_vec(),
                })
            }
            _ => None,
        }
    }
}

fn encode_operation(
    tag: u8,
    path: &str,
    tail_len: usize,
    append_tail: impl FnOnce(&mut Vec<u8>) -> Result<(), FileExit>,
) -> Result<PreparedRequest, FileExit> {
    let path_len = u32::try_from(path.len()).map_err(|_| FileExit::InvalidPath)?;
    let mut encoded = Vec::new();
    encoded
        .try_reserve(
            REQUEST_MAGIC
                .len()
                .saturating_add(1)
                .saturating_add(4)
                .saturating_add(path.len())
                .saturating_add(tail_len),
        )
        .map_err(|_| FileExit::IoFailed)?;
    encoded.extend_from_slice(REQUEST_MAGIC);
    encoded.push(tag);
    encoded.extend_from_slice(&path_len.to_be_bytes());
    encoded.extend_from_slice(path.as_bytes());
    append_tail(&mut encoded)?;
    Ok(PreparedRequest::seal(encoded))
}

fn take(input: &[u8], length: usize) -> Option<(&[u8], &[u8])> {
    (input.len() >= length).then(|| input.split_at(length))
}

fn take_u32(input: &[u8]) -> Option<(u32, &[u8])> {
    let (bytes, rest) = take(input, 4)?;
    let mut fixed = [0u8; 4];
    fixed.copy_from_slice(bytes);
    Some((u32::from_be_bytes(fixed), rest))
}

fn take_u64(input: &[u8]) -> Option<(u64, &[u8])> {
    let (bytes, rest) = take(input, 8)?;
    let mut fixed = [0u8; 8];
    fixed.copy_from_slice(bytes);
    Some((u64::from_be_bytes(fixed), rest))
}

fn validate_logical_path(path: &str) -> Result<String, FileExit> {
    let bytes = path.as_bytes();
    let has_windows_drive_prefix =
        bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if path.is_empty()
        || path.starts_with('/')
        || has_windows_drive_prefix
        || path.contains('\0')
        || path.contains('\\')
        || Path::new(path).is_absolute()
        || path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(FileExit::InvalidPath);
    }
    Ok(String::from(path))
}

fn execute(
    root: &Path,
    operation: FileOperation,
    cancelled: &AtomicBool,
    call_id: crate::boundary::CallId,
) -> Option<Completion> {
    if cancelled.load(Ordering::Acquire) {
        return None;
    }
    match operation {
        FileOperation::Read { path, max_read } => {
            execute_read(&root.join(path), max_read, cancelled)
        }
        FileOperation::Replace { path, bytes } => {
            execute_replace(&root.join(path), &bytes, cancelled, call_id)
        }
    }
}

fn execute_read(path: &Path, max_read: u64, cancelled: &AtomicBool) -> Option<Completion> {
    let mut file = match FsFile::open(path) {
        Ok(file) => file,
        Err(error) => {
            return Some(Completion::file_exit(classify_open_failure(path, &error)));
        }
    };
    match file.metadata() {
        Ok(metadata) if !metadata.is_file() => {
            return Some(Completion::file_exit(FileExit::NotAFile));
        }
        Ok(metadata) if metadata.len() > max_read => {
            return Some(Completion::file_exit(FileExit::ReadLimitExceeded {
                limit: max_read,
            }));
        }
        Ok(_) => {}
        Err(error) => return Some(Completion::file_exit(classify_io(&error))),
    }

    let mut bytes = Vec::new();
    let mut chunk = [0u8; IO_CHUNK_BYTES];
    loop {
        if cancelled.load(Ordering::Acquire) {
            return None;
        }
        match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                let observed = u64::try_from(bytes.len())
                    .unwrap_or(u64::MAX)
                    .saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
                if observed > max_read {
                    return Some(Completion::file_exit(FileExit::ReadLimitExceeded {
                        limit: max_read,
                    }));
                }
                if bytes.try_reserve(read).is_err() {
                    return Some(Completion::file_exit(FileExit::IoFailed));
                }
                bytes.extend_from_slice(&chunk[..read]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Some(Completion::file_exit(classify_io(&error))),
        }
    }
    if cancelled.load(Ordering::Acquire) {
        return None;
    }
    Some(Completion::file(
        Some(Data::encoded(Encoding::Raw, bytes)),
        FileExit::Completed,
    ))
}

fn classify_open_failure(path: &Path, error: &std::io::Error) -> FileExit {
    match std::fs::metadata(path) {
        Ok(metadata) if !metadata.is_file() => FileExit::NotAFile,
        _ => classify_io(error),
    }
}

fn execute_replace(
    target: &Path,
    bytes: &[u8],
    cancelled: &AtomicBool,
    call_id: crate::boundary::CallId,
) -> Option<Completion> {
    if let Ok(metadata) = std::fs::metadata(target)
        && !metadata.is_file()
    {
        return Some(Completion::file_exit(FileExit::NotAFile));
    }
    let Some(parent) = target.parent() else {
        return Some(Completion::file_exit(FileExit::InvalidPath));
    };
    let (staging, mut file) = match open_staging(parent, target, call_id) {
        Ok(staging) => staging,
        Err(exit) => return Some(Completion::file_exit(exit)),
    };

    for chunk in bytes.chunks(IO_CHUNK_BYTES) {
        if cancelled.load(Ordering::Acquire) {
            drop(file);
            let _ = std::fs::remove_file(&staging);
            return None;
        }
        if let Err(error) = file.write_all(chunk) {
            drop(file);
            let _ = std::fs::remove_file(&staging);
            return Some(Completion::file_exit(classify_io(&error)));
        }
    }
    drop(file);
    if cancelled.load(Ordering::Acquire) {
        let _ = std::fs::remove_file(&staging);
        return None;
    }

    match atomic_replace(&staging, target) {
        Ok(()) => Some(Completion::file_exit(FileExit::Completed)),
        Err(exit) => {
            let _ = std::fs::remove_file(&staging);
            Some(Completion::file_exit(exit))
        }
    }
}

fn open_staging(
    parent: &Path,
    target: &Path,
    call_id: crate::boundary::CallId,
) -> Result<(PathBuf, FsFile), FileExit> {
    // `create_new` makes collision handling non-destructive. The explicit
    // target inequality also protects the commit boundary when a user-chosen
    // filename happens to resemble our private staging convention.
    for nonce in 0u8..STAGING_ATTEMPTS {
        let staging = parent.join(format!(".rastok-{call_id}-{nonce}.tmp"));
        if staging == target {
            continue;
        }
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)
        {
            Ok(file) => return Ok((staging, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(classify_io(&error)),
        }
    }
    Err(FileExit::IoFailed)
}

#[cfg(unix)]
fn atomic_replace(staging: &Path, target: &Path) -> Result<(), FileExit> {
    std::fs::rename(staging, target).map_err(|error| classify_replace(&error))
}

#[cfg(windows)]
fn atomic_replace(staging: &Path, target: &Path) -> Result<(), FileExit> {
    if target.exists() {
        return Err(FileExit::AtomicReplaceUnavailable);
    }
    std::fs::rename(staging, target).map_err(|error| classify_replace(&error))
}

#[cfg(not(any(unix, windows)))]
fn atomic_replace(_staging: &Path, _target: &Path) -> Result<(), FileExit> {
    Err(FileExit::AtomicReplaceUnavailable)
}

fn classify_replace(error: &std::io::Error) -> FileExit {
    match error.kind() {
        std::io::ErrorKind::Unsupported | std::io::ErrorKind::CrossesDevices => {
            FileExit::AtomicReplaceUnavailable
        }
        _ => classify_io(error),
    }
}

fn classify_io(error: &std::io::Error) -> FileExit {
    match error.kind() {
        std::io::ErrorKind::NotFound => FileExit::NotFound,
        std::io::ErrorKind::PermissionDenied => FileExit::PermissionDenied,
        std::io::ErrorKind::IsADirectory | std::io::ErrorKind::NotADirectory => FileExit::NotAFile,
        std::io::ErrorKind::InvalidInput
        | std::io::ErrorKind::InvalidData
        | std::io::ErrorKind::InvalidFilename => FileExit::InvalidPath,
        _ => FileExit::IoFailed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_framing_is_exact_and_total() {
        let operations = [
            FileOperation::Read {
                path: String::from("a/b.bin"),
                max_read: 42,
            },
            FileOperation::Replace {
                path: String::from("a/b.bin"),
                bytes: vec![0, 1, 255],
            },
        ];
        for operation in operations {
            let prepared = operation.encode().expect("the small operation frames");
            assert_eq!(FileOperation::decode(prepared.bytes()), Some(operation));
            let mut trailing = prepared.bytes().to_vec();
            trailing.push(0);
            assert_eq!(FileOperation::decode(&trailing), None);
        }
        let left = FileOperation::Read {
            path: String::from("a"),
            max_read: 42,
        }
        .encode()
        .expect("the left request frames");
        let right = FileOperation::Read {
            path: String::from("b"),
            max_read: 42,
        }
        .encode()
        .expect("the right request frames");
        assert_ne!(left.digest(), right.digest(), "the path is call identity");
    }

    #[test]
    fn logical_paths_are_relative_and_portable() {
        assert_eq!(
            validate_logical_path("dir/file"),
            Ok(String::from("dir/file"))
        );
        for path in [
            "",
            "/file",
            "C:secret.txt",
            "z:dir/file",
            "a//b",
            "./a",
            "a/../b",
            "a\\b",
            "a\0b",
        ] {
            assert_eq!(validate_logical_path(path), Err(FileExit::InvalidPath));
        }
    }

    #[test]
    fn opening_a_directory_has_one_portable_terminal_case() {
        let root = std::env::temp_dir().join(format!(
            "rastok-file-open-classification-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the test directory is writable");
        let permission_denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);

        assert_eq!(
            classify_open_failure(&root, &permission_denied),
            FileExit::NotAFile,
            "Windows may refuse opening a directory before metadata is available from the handle"
        );
        std::fs::remove_dir_all(root).expect("the test directory is removable");
    }

    #[test]
    fn scheduler_facts_inhabit_the_file_exit_contract() {
        let ty = crate::boundary::file_exit_type().expect("the FileExit type is valid");
        for completion in [
            crate::boundary::file_scheduler_completion(SchedulerCompletion::TimeoutExpired),
            crate::boundary::file_scheduler_completion(SchedulerCompletion::OutcomeUnknown),
        ] {
            let exit = &completion.outputs()[0].data;
            let Data::Materialized(value) = exit else {
                panic!("File exits are materialized");
            };
            assert!(ty.accepts(value));
        }
    }

    #[test]
    fn cancellation_before_observation_or_commit_is_silent() {
        let root = std::env::temp_dir().join(format!("rastok-file-cancel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the test root is writable");
        let target = root.join("artifact.bin");
        std::fs::write(&target, b"before").expect("the seed writes");
        let cancelled = AtomicBool::new(true);
        let call_id = crate::boundary::CallId::new(crate::envelope::Id::from_bytes([7; 16]));

        let completion = execute(
            &root,
            FileOperation::Replace {
                path: String::from("artifact.bin"),
                bytes: b"after".to_vec(),
            },
            &cancelled,
            call_id,
        );

        assert!(completion.is_none());
        assert_eq!(
            std::fs::read(&target).expect("the target remains readable"),
            b"before"
        );
        std::fs::remove_dir_all(root).expect("the test root cleans up");
    }

    #[test]
    fn a_user_filename_can_never_become_its_own_staging_file() {
        let root = std::env::temp_dir().join(format!(
            "rastok-file-staging-collision-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the test root is writable");
        let call_id = crate::boundary::CallId::new(crate::envelope::Id::from_bytes([8; 16]));
        let filename = format!(".rastok-{call_id}-0.tmp");
        let target = root.join(&filename);
        std::fs::write(&target, b"before").expect("the seed writes");

        let completion = execute(
            &root,
            FileOperation::Replace {
                path: filename,
                bytes: b"after".to_vec(),
            },
            &AtomicBool::new(false),
            call_id,
        );

        assert!(completion.is_some());
        assert_eq!(std::fs::read(&target).expect("the target reads"), b"after");
        std::fs::remove_dir_all(root).expect("the test root cleans up");
    }

    #[test]
    fn permission_errors_have_one_portable_projection() {
        let error = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(classify_io(&error), FileExit::PermissionDenied);
    }
}
