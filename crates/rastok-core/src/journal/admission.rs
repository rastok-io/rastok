//! Admission queries against either the live journal or an exact replay
//! prefix. Looking at a later `l:stop` while rebuilding an earlier route
//! would silently move that delivery into a successor generation.

use super::*;
use crate::instance;

/// Borrowed dynamically by routing so live and prefix recovery share one
/// implementation without making later journal facts visible to the prefix.
pub(crate) trait InstanceAdmissions {
    fn instance_generation_for_admission(
        &self,
        address: &instance::Identity,
    ) -> Result<instance::Generation, Error>;
    fn instance_commands(&self) -> Result<instance_stop::InstanceCommandView, Error>;
}

impl<T: Journal + ?Sized> InstanceAdmissions for &T {
    fn instance_generation_for_admission(
        &self,
        address: &instance::Identity,
    ) -> Result<instance::Generation, Error> {
        Journal::instance_generation_for_admission(*self, address)
    }
    fn instance_commands(&self) -> Result<instance_stop::InstanceCommandView, Error> {
        Journal::instance_commands(*self)
    }
}

/// The single admission-generation fold, held live or fed an exact prefix.
/// It carries no authority beyond the accepted lifecycle records.
#[derive(Debug, Default)]
pub(super) struct Fold {
    instance_lifecycles: BTreeMap<instance::Identity, InstanceLifecycle>,
    /// Fast rejection: ordinary admission need not walk ancestors if none close.
    active_close_roots: BTreeSet<instance::Generation>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct InstanceLifecycle {
    starts: u64,
    active: bool,
    parent: Option<instance::Generation>,
    closing: bool,
}

impl Fold {
    /// Cached projection of the pure lifecycle fold for one logical address.
    /// `starts` is also the next generation ordinal whenever no generation is
    /// active. A durable implementation rebuilds the same projection while
    /// replaying `l:start`/`l:stop`; no Worker-local counter participates.
    fn instance_lifecycle(&self, address: &instance::Identity) -> InstanceLifecycle {
        self.instance_lifecycles
            .get(address)
            .cloned()
            .unwrap_or_default()
    }

    pub(super) fn active_instance_ref(
        &self,
        address: &instance::Identity,
    ) -> Option<instance::Generation> {
        let lifecycle = self.instance_lifecycle(address);
        lifecycle
            .active
            .then(|| instance::Generation::new(address.clone(), lifecycle.starts.saturating_sub(1)))
    }

    pub(super) fn instance_is_within(
        &self,
        child: &instance::Generation,
        ancestor: &instance::Generation,
    ) -> bool {
        let mut cursor = Some(child.clone());
        while let Some(reference) = cursor {
            if reference == *ancestor {
                return true;
            }
            let lifecycle = self.instance_lifecycle(reference.address());
            if !lifecycle.active || lifecycle.starts.saturating_sub(1) != reference.generation() {
                return false;
            }
            cursor = lifecycle.parent;
        }
        false
    }

    /// Outermost durable close root governing this active generation.
    pub(super) fn closing_scope(
        &self,
        instance: &instance::Generation,
    ) -> Option<instance::Generation> {
        if self.active_close_roots.is_empty() {
            return None;
        }
        let mut cursor = Some(instance.clone());
        let mut found = None;
        while let Some(reference) = cursor {
            let lifecycle = self.instance_lifecycle(reference.address());
            if !lifecycle.active || lifecycle.starts.saturating_sub(1) != reference.generation() {
                return found;
            }
            if lifecycle.closing {
                found = Some(reference.clone());
            }
            cursor = lifecycle.parent;
        }
        found
    }

    pub(super) fn has_active_children(&self, parent: &instance::Generation) -> bool {
        self.instance_lifecycles
            .values()
            .any(|lifecycle| lifecycle.active && lifecycle.parent.as_ref() == Some(parent))
    }

    pub(super) fn instance_generation_for_admission(
        &self,
        address: &instance::Identity,
    ) -> Result<instance::Generation, Error> {
        let lifecycle = self.instance_lifecycle(address);
        let closing = lifecycle.active
            && !self.active_close_roots.is_empty()
            && self
                .closing_scope(&instance::Generation::new(
                    address.clone(),
                    lifecycle.starts.saturating_sub(1),
                ))
                .is_some();
        let generation = if lifecycle.active && !closing {
            lifecycle.starts.saturating_sub(1)
        } else {
            lifecycle.starts
        };
        Ok(instance::Generation::new(address.clone(), generation))
    }

    /// Check the generation before the journal's remaining acceptance checks.
    /// Keep this snapshot private to the journal until the whole record commits.
    pub(super) fn prepare(&self, record: &TransactionRecord) -> Result<InstanceLifecycle, Error> {
        let lifecycle = self.instance_lifecycle(record.instance.address());
        let is_start = record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Start);
        let expected = instance::Generation::new(
            record.instance.address().clone(),
            if is_start {
                lifecycle.starts
            } else {
                lifecycle.starts.saturating_sub(1)
            },
        );
        if record.instance != expected {
            return Err(Error::InstanceGenerationMismatch {
                expected,
                got: record.instance.clone(),
            });
        }
        if is_start == lifecycle.active {
            return Err(Error::InvalidInstanceLifecycle {
                instance: record.instance.clone(),
                active: lifecycle.active,
            });
        }
        Ok(lifecycle)
    }

    /// Apply only after all other journal checks have succeeded. An overflow
    /// refuses before any index mutation; nothing fallible follows this update.
    pub(super) fn commit(
        &mut self,
        record: &TransactionRecord,
        mut lifecycle: InstanceLifecycle,
    ) -> Result<(), Error> {
        if record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Start) {
            lifecycle.starts = lifecycle
                .starts
                .checked_add(1)
                .ok_or(Error::SequenceOverflow)?;
            lifecycle.active = true;
            lifecycle.parent = record.parent_instance.clone();
            lifecycle.closing = false;
        } else if record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Stop) {
            self.active_close_roots.remove(&record.instance);
            lifecycle.active = false;
            lifecycle.parent = None;
            lifecycle.closing = false;
        } else if record.close_scope.is_some() {
            self.active_close_roots.insert(record.instance.clone());
            lifecycle.closing = true;
        }
        self.instance_lifecycles
            .insert(record.instance.address().clone(), lifecycle);
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct AdmissionPrefix {
    admissions: Fold,
    instance_commands: Option<Box<instance_stop::InstanceCommandView>>,
}

impl AdmissionPrefix {
    pub(crate) fn retaining_instance_commands(enabled: bool) -> Self {
        Self {
            instance_commands: enabled
                .then(|| Box::new(instance_stop::InstanceCommandView::default())),
            ..Self::default()
        }
    }

    pub(crate) fn apply(&mut self, record: &Record) -> Result<(), Error> {
        if let Some(fold) = self.instance_commands.as_deref_mut() {
            fold.apply(record).map_err(Error::InvalidInstanceCommand)?;
        }
        if let Record::Transaction(record) = record {
            let lifecycle = self.admissions.prepare(record)?;
            self.admissions.commit(record, lifecycle)?;
        }
        Ok(())
    }
}

impl InstanceAdmissions for AdmissionPrefix {
    fn instance_generation_for_admission(
        &self,
        address: &instance::Identity,
    ) -> Result<instance::Generation, Error> {
        self.admissions.instance_generation_for_admission(address)
    }

    fn instance_commands(&self) -> Result<instance_stop::InstanceCommandView, Error> {
        self.instance_commands
            .as_deref()
            .cloned()
            .ok_or(Error::InvalidRetainedRecord(
                "instance command prefix was not enabled",
            ))
    }
}
