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
    fn instance_commands(&self) -> Result<instance_stop::Fold, Error>;
}

impl<T: Journal + ?Sized> InstanceAdmissions for &T {
    fn instance_generation_for_admission(
        &self,
        address: &instance::Identity,
    ) -> Result<instance::Generation, Error> {
        Journal::instance_generation_for_admission(*self, address)
    }
    fn instance_commands(&self) -> Result<instance_stop::Fold, Error> {
        Journal::instance_commands(*self)
    }
}

#[derive(Default)]
pub(crate) struct AdmissionPrefix {
    lifecycles: BTreeMap<instance::Identity, InstanceLifecycle>,
    instance_commands: Option<Box<instance_stop::Fold>>,
}

impl AdmissionPrefix {
    pub(crate) fn retaining_instance_commands(enabled: bool) -> Self {
        Self {
            instance_commands: enabled.then(|| Box::new(instance_stop::Fold::default())),
            ..Self::default()
        }
    }

    pub(crate) fn apply(&mut self, record: &Record) -> Result<(), Error> {
        if let Some(fold) = self.instance_commands.as_deref_mut() {
            fold.apply(record).map_err(Error::InvalidInstanceCommand)?;
        }
        let Record::Transaction(record) = record else {
            return Ok(());
        };
        let lifecycle = self
            .lifecycles
            .entry(record.instance.address().clone())
            .or_default();
        let is_start = record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Start);
        let expected = instance::Generation::new(
            record.instance.address().clone(),
            if is_start {
                lifecycle.starts
            } else {
                lifecycle.starts.saturating_sub(1)
            },
        );
        if expected != record.instance {
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
        if is_start {
            lifecycle.starts = lifecycle
                .starts
                .checked_add(1)
                .ok_or(Error::SequenceOverflow)?;
            lifecycle.active = true;
            lifecycle.parent = record.parent_instance.clone();
            lifecycle.closing = false;
        } else if record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Stop) {
            lifecycle.active = false;
            lifecycle.parent = None;
            lifecycle.closing = false;
        } else if record.close_scope.is_some() {
            lifecycle.closing = true;
        }
        Ok(())
    }

    fn is_closing(&self, reference: &instance::Generation) -> bool {
        let mut cursor = Some(reference);
        while let Some(reference) = cursor {
            let Some(state) = self.lifecycles.get(reference.address()) else {
                return false;
            };
            if !state.active || state.starts.saturating_sub(1) != reference.generation() {
                return false;
            }
            if state.closing {
                return true;
            }
            cursor = state.parent.as_ref();
        }
        false
    }
}

impl InstanceAdmissions for AdmissionPrefix {
    fn instance_generation_for_admission(
        &self,
        address: &instance::Identity,
    ) -> Result<instance::Generation, Error> {
        let Some(state) = self.lifecycles.get(address) else {
            return Ok(instance::Generation::new(address.clone(), 0));
        };
        let current = instance::Generation::new(address.clone(), state.starts.saturating_sub(1));
        if state.active && !self.is_closing(&current) {
            Ok(current)
        } else {
            Ok(instance::Generation::new(address.clone(), state.starts))
        }
    }

    fn instance_commands(&self) -> Result<instance_stop::Fold, Error> {
        self.instance_commands
            .as_deref()
            .cloned()
            .ok_or(Error::InvalidRetainedRecord(
                "instance command prefix was not enabled",
            ))
    }
}
