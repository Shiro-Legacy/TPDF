// SPDX-License-Identifier: AGPL-3.0-or-later
//! The operation journal, with the undo and redo semantics engine.ts relies
//! on, over exact object and trailer snapshots.
//!
//! An operation records the state of each object and of the trailer before
//! its first touch. Ending an operation that touched nothing records no step;
//! otherwise the redo branch is dropped and the step stored. Abandoning
//! restores the befores, and drops the redo branch only when something was
//! touched.
//!
//! A step holds one state per object, the one the document does not: the
//! befores while it is applied, the afters once it is undone. Undo and redo
//! exchange them with the document's, O(objects in the step) with nothing
//! copied, and states share every dictionary an edit did not reach with
//! the live document (`Dictionary` is copy on write), so a step costs what
//! its edits changed.
use std::collections::BTreeMap;

use crate::document::{Document, DocumentError, ObjectState, TrailerState};
use crate::object::ObjectId;

#[derive(Clone, Debug)]
struct ObjectChange {
    id: ObjectId,
    /// The state the document does not hold: before the step while it is
    /// applied, after it once undone.
    other: Option<ObjectState>,
}

#[derive(Clone, Debug)]
struct Step {
    name: String,
    objects: Vec<ObjectChange>,
    /// The trailer state the document does not hold, as for objects.
    trailer: Option<TrailerState>,
    /// Never reused, so a save ticket can name the state after this step.
    serial: u64,
}

/// The operation being recorded.
#[derive(Debug)]
struct Pending {
    name: String,
    objects: BTreeMap<ObjectId, Option<ObjectState>>,
    trailer: Option<TrailerState>,
}

/// What a journal move changed, for caches keyed on document content.
#[derive(Debug, Default)]
pub(crate) struct Changed {
    pub(crate) objects: Vec<ObjectId>,
    pub(crate) trailer: bool,
}

impl Changed {
    pub(crate) fn is_empty(&self) -> bool {
        self.objects.is_empty() && !self.trailer
    }
}

#[derive(Debug, Default)]
pub struct Journal {
    steps: Vec<Step>,
    position: usize,
    pending: Option<Pending>,
    serial: u64,
}

impl Journal {
    /// Steps applied: the journal's position.
    pub fn position(&self) -> usize {
        self.position
    }
    pub fn can_undo(&self) -> bool {
        self.position > 0
    }
    pub fn can_redo(&self) -> bool {
        self.position < self.steps.len()
    }
    /// Every step name, oldest first, the redo branch included.
    pub fn steps(&self) -> impl Iterator<Item = &str> {
        self.steps.iter().map(|step| step.name.as_str())
    }
    /// An operation is being recorded.
    pub fn is_open(&self) -> bool {
        self.pending.is_some()
    }
    /// Identifies the current state among every state this journal has had:
    /// 0 for the opened document, otherwise the serial of the last applied step.
    pub(crate) fn state_serial(&self) -> u64 {
        self.position
            .checked_sub(1)
            .and_then(|last| self.steps.get(last))
            .map_or(0, |step| step.serial)
    }
    /// The position whose state `state_serial` named, if it is still reachable.
    pub(crate) fn position_of(&self, serial: u64) -> Option<usize> {
        if serial == 0 {
            return Some(0);
        }
        let index = self.steps.iter().position(|step| step.serial == serial)?;
        Some(index + 1)
    }

    /// Starts recording; false when an operation is already open.
    pub(crate) fn begin(&mut self, name: &str) -> bool {
        if self.pending.is_some() {
            return false;
        }
        self.pending = Some(Pending {
            name: name.to_string(),
            objects: BTreeMap::new(),
            trailer: None,
        });
        true
    }

    /// Records `id`'s state before its first touch in the open operation.
    /// True when this was the first touch. False without an open operation.
    pub(crate) fn touch(&mut self, doc: &Document<'_>, id: ObjectId) -> bool {
        let Some(pending) = &mut self.pending else {
            return false;
        };
        if pending.objects.contains_key(&id) {
            return false;
        }
        pending.objects.insert(id, doc.snapshot(id));
        true
    }
    /// Forgets a first touch whose write failed, unless it changed something.
    pub(crate) fn untouch_if_unchanged(&mut self, doc: &Document<'_>, id: ObjectId) {
        if let Some(pending) = &mut self.pending {
            if pending
                .objects
                .get(&id)
                .is_some_and(|before| doc.is_in_state(id, before.as_ref()))
            {
                pending.objects.remove(&id);
            }
        }
    }
    pub(crate) fn touch_trailer(&mut self, doc: &Document<'_>) -> bool {
        match &mut self.pending {
            Some(pending) if pending.trailer.is_none() => {
                pending.trailer = Some(doc.trailer_state());
                true
            }
            _ => false,
        }
    }
    pub(crate) fn untouch_trailer(&mut self) {
        if let Some(pending) = &mut self.pending {
            pending.trailer = None;
        }
    }
    /// Every object state the journal holds (both branches and the open
    /// operation), `None` for an object absent in it, and every trailer
    /// state: what undo, redo or abandon can bring back.
    pub(crate) fn held(
        &self,
    ) -> (
        impl Iterator<Item = (ObjectId, Option<&ObjectState>)> + '_,
        impl Iterator<Item = &TrailerState> + '_,
    ) {
        let steps = (self.steps.iter())
            .flat_map(|step| step.objects.iter())
            .map(|change| (change.id, change.other.as_ref()));
        let pending = (self.pending.iter())
            .flat_map(|pending| pending.objects.iter())
            .map(|(&id, state)| (id, state.as_ref()));
        let trailers = (self.steps.iter())
            .filter_map(|step| step.trailer.as_ref())
            .chain(self.pending.iter().filter_map(|p| p.trailer.as_ref()));
        (steps.chain(pending), trailers)
    }
    /// Objects the open operation has touched.
    pub(crate) fn pending_objects(&self) -> impl Iterator<Item = ObjectId> + '_ {
        self.pending
            .iter()
            .flat_map(|pending| pending.objects.keys().copied())
    }
    /// Whether the open operation has touched the trailer.
    pub(crate) fn pending_trailer(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.trailer.is_some())
    }

    /// Records the open operation as a step, unless it touched nothing.
    pub(crate) fn end(&mut self) -> Changed {
        let Some(pending) = self.pending.take() else {
            return Changed::default();
        };
        let mut changed = Changed {
            objects: pending.objects.keys().copied().collect(),
            trailer: pending.trailer.is_some(),
        };
        if changed.is_empty() {
            return changed;
        }
        let objects = (pending.objects.into_iter())
            .map(|(id, before)| ObjectChange { id, other: before })
            .collect();
        self.steps.truncate(self.position);
        self.serial += 1;
        self.steps.push(Step {
            name: pending.name,
            objects,
            trailer: pending.trailer,
            serial: self.serial,
        });
        self.position = self.steps.len();
        changed.objects.sort_unstable();
        changed
    }

    /// Puts back everything the open operation touched. A write drops the
    /// redo branch.
    pub(crate) fn abandon(
        &mut self,
        doc: &mut Document<'static>,
    ) -> Result<Changed, DocumentError> {
        let Some(pending) = self.pending.take() else {
            return Ok(Changed::default());
        };
        let changed = Changed {
            objects: pending.objects.keys().copied().collect(),
            trailer: pending.trailer.is_some(),
        };
        for (id, before) in pending.objects {
            doc.restore(id, before)?;
        }
        if let Some(before) = pending.trailer {
            doc.restore_trailer(before)?;
        }
        if !changed.is_empty() {
            self.steps.truncate(self.position);
        }
        Ok(changed)
    }

    /// Reverts the last applied step; its name, or None with nothing to undo.
    pub(crate) fn undo(
        &mut self,
        doc: &mut Document<'static>,
    ) -> Result<Option<(String, Changed)>, DocumentError> {
        if self.pending.is_some() {
            return Ok(None);
        }
        let Some(index) = self.position.checked_sub(1) else {
            return Ok(None);
        };
        let Some(step) = self.steps.get_mut(index) else {
            return Ok(None);
        };
        for change in step.objects.iter_mut().rev() {
            doc.exchange(change.id, &mut change.other)?;
        }
        if let Some(other) = &mut step.trailer {
            doc.exchange_trailer(other)?;
        }
        self.position = index;
        Ok(Some((step.name.clone(), changed(step))))
    }

    /// Reapplies the next step of the redo branch.
    pub(crate) fn redo(
        &mut self,
        doc: &mut Document<'static>,
    ) -> Result<Option<(String, Changed)>, DocumentError> {
        if self.pending.is_some() {
            return Ok(None);
        }
        let Some(step) = self.steps.get_mut(self.position) else {
            return Ok(None);
        };
        for change in &mut step.objects {
            doc.exchange(change.id, &mut change.other)?;
        }
        if let Some(other) = &mut step.trailer {
            doc.exchange_trailer(other)?;
        }
        self.position += 1;
        Ok(Some((step.name.clone(), changed(step))))
    }
}

fn changed(step: &Step) -> Changed {
    Changed {
        objects: step.objects.iter().map(|change| change.id).collect(),
        trailer: step.trailer.is_some(),
    }
}
