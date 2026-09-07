//! Admission queries against either the live journal or an exact replay
//! prefix. Looking at a later `l:stop` while rebuilding an earlier route
//! would silently move that delivery into a successor generation.

use super::*;

pub(crate) trait InstanceAdmissions {
    fn instance_ref_for_admission(&self, address: &InstanceAddress) -> Result<InstanceRef, Error>;
}

impl<T: Journal + ?Sized> InstanceAdmissions for T {
    fn instance_ref_for_admission(&self, address: &InstanceAddress) -> Result<InstanceRef, Error> {
        Journal::instance_ref_for_admission(self, address)
    }
}

#[derive(Default)]
pub(crate) struct AdmissionPrefix {
    lifecycles: BTreeMap<InstanceAddress, InstanceLifecycle>,
}

impl AdmissionPrefix {
    pub(crate) fn apply(&mut self, record: &Record) -> Result<(), Error> {
        let Record::Transaction(record) = record else {
            return Ok(());
        };
        let lifecycle = self
            .lifecycles
            .entry(record.instance.address().clone())
            .or_default();
        let is_start = record.trigger == node::Trigger::Lifecycle(node::Lifecycle::Start);
        let expected = InstanceRef::new(
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

    fn is_closing(&self, reference: &InstanceRef) -> bool {
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
    fn instance_ref_for_admission(&self, address: &InstanceAddress) -> Result<InstanceRef, Error> {
        let Some(state) = self.lifecycles.get(address) else {
            return Ok(InstanceRef::new(address.clone(), 0));
        };
        let current = InstanceRef::new(address.clone(), state.starts.saturating_sub(1));
        if state.active && !self.is_closing(&current) {
            Ok(current)
        } else {
            Ok(InstanceRef::new(address.clone(), state.starts))
        }
    }
}
