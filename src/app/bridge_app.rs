//! Dormant App routing to the bridge glue (cutover step S3).
//!
//! `main` never attaches a glue, so the legacy Teams paths stay in use until
//! the consented startup switch (S4). With a glue attached, every Webex read
//! and write goes through it instead of `dispatch_to_teams`:
//! - send/reply/edit submit the editor's explicit `DraftSnapshot`; the editor
//!   is cleared only by a confirmed success of that exact composition;
//! - delete and leave are explicit single-slot actions; a multi-room leave is
//!   queued and submitted one room at a time as the slot frees;
//! - opening a room loads its newest page once; moving above the first row
//!   loads the older page; selecting a missing-root placeholder loads that
//!   thread;
//! - every tick drains the glue (§7.2 d), re-anchors the selection by row ID
//!   and refreshes the one-line status (session, daemon/Webex, stale data,
//!   write outcome); `x` dismisses a failed/uncertain write review.

use super::{
    bridge_glue::{Action as GlueAction, ActionOutcome, BridgeGlue, QuitMark, WriteRefusal},
    cache::room::RoomId,
    state::BridgeView,
    App,
};
use crate::{
    bridge::session::SessionPhase,
    frontend::{
        data::MessageId, editor_submission::SubmissionError, editor_write::EditorWriteStatus,
        events::ConnectionState, message_view::ViewRow,
    },
};
use color_eyre::{eyre::eyre, Result};

/// Runtime events reduced per App tick.
const DRAIN_PER_TICK: usize = 256;

impl App<'_> {
    /// Attach the bridge glue; from now on Webex reads/writes use it.
    pub(crate) fn attach_bridge(&mut self, glue: BridgeGlue) {
        self.bridge = Some(glue);
        self.refresh_bridge_view();
    }

    pub(crate) fn has_bridge(&self) -> bool {
        self.bridge.is_some()
    }

    pub(crate) fn bridge(&self) -> Option<&BridgeGlue> {
        self.bridge.as_ref()
    }

    /// Drain runtime events, advance queued leaves, keep the selection on the
    /// same row and refresh the status line.
    pub(super) fn bridge_tick(&mut self) {
        let Some(glue) = self.bridge.as_mut() else {
            return;
        };
        let anchor = selected_row_id(&self.state);
        glue.drain(&mut self.state, DRAIN_PER_TICK);
        // While quitting, queued leaves stay queued (reported as not sent).
        if let Some(room) = self
            .pending_leaves
            .front()
            .cloned()
            .filter(|_| !self.quitting)
        {
            match glue.leave_room(&room) {
                Ok(()) => {
                    self.pending_leaves.pop_front();
                }
                // Wait for the slot or the session; anything else drops it.
                Err(WriteRefusal::Busy | WriteRefusal::NotReady) => {}
                Err(_) => {
                    self.pending_leaves.pop_front();
                }
            }
        }
        let rows = self.state.num_messages_active_room();
        self.state.messages_list.set_nb_messages(rows);
        if let Some(id) = anchor {
            if let Some(index) = row_index(&self.state, &id) {
                self.state.messages_list.select_index(index);
            }
        }
        self.refresh_bridge_view();
    }

    /// Submit the editor's composition (send, reply or edit).
    pub(super) fn bridge_send(&mut self) -> Result<()> {
        let glue = self.bridge.as_mut().ok_or(eyre!("no bridge"))?;
        if self.state.message_editor.is_empty() {
            return Err(eyre!("An empty message cannot be sent."));
        }
        let active = self
            .state
            .rooms_list
            .active_room_id()
            .map(|id| crate::frontend::data::RoomId::new(id.as_str()));
        let draft = self
            .state
            .message_editor
            .draft(active.as_ref())
            .ok_or(eyre!("Cannot send message, no room selected."))?;
        let result = glue.submit_draft(&self.state, &draft);
        self.refresh_bridge_view();
        result.map_err(|refusal| eyre!(refusal_text(refusal)))?;
        if let Some(room) = self.state.rooms_list.active_room_id().cloned() {
            self.state.cache.rooms.mark_read(&room);
        }
        Ok(())
    }

    pub(super) fn bridge_delete(&mut self, room: &RoomId, message: &MessageId) -> Result<()> {
        let glue = self.bridge.as_mut().ok_or(eyre!("no bridge"))?;
        let result = glue.delete_message(room, message);
        self.refresh_bridge_view();
        result.map_err(|refusal| eyre!(refusal_text(refusal)))
    }

    /// Queue leaves; they are submitted one at a time on later ticks.
    pub(super) fn bridge_leave_rooms(&mut self, rooms: Vec<RoomId>) {
        for room in rooms {
            if !self.pending_leaves.contains(&room) {
                self.pending_leaves.push_back(room);
            }
        }
        self.bridge_tick();
    }

    pub(super) fn bridge_open_room(&mut self, room: &RoomId) {
        if let Some(glue) = self.bridge.as_mut() {
            glue.open_room(&mut self.state, room);
        }
    }

    pub(super) fn bridge_load_older(&mut self, room: &RoomId) {
        if let Some(glue) = self.bridge.as_mut() {
            glue.load_older(room);
        }
    }

    /// Selecting a missing-root placeholder loads that thread (explicit).
    pub(super) fn bridge_selection_changed(&mut self) {
        let Some(room) = self.state.rooms_list.active_room_id().cloned() else {
            return;
        };
        let Some(index) = self.state.messages_list.selected_index() else {
            return;
        };
        let root = match self.state.cache.rows_in_room(&room).nth(index) {
            Some(ViewRow::MissingRoot(id)) => id.clone(),
            _ => return,
        };
        if let Some(glue) = self.bridge.as_mut() {
            glue.load_thread(&room, &root);
        }
        self.state.update_actions(self.state.active_pane.clone());
    }

    pub(super) fn bridge_dismiss_review(&mut self) {
        if let Some(glue) = self.bridge.as_mut() {
            glue.dismiss_review();
        }
        self.refresh_bridge_view();
    }

    /// Quit path step 1: note in-flight writes and request runtime shutdown.
    /// The caller then joins the runtime (≤ 35 s), ticks once more, restores
    /// the terminal and prints `quit_summary`.
    pub(crate) fn begin_quit(&mut self) -> Option<QuitMark> {
        self.quitting = true;
        self.bridge.as_mut().map(BridgeGlue::begin_quit)
    }

    /// Metadata-only lines to print after the terminal is restored.
    pub(crate) fn quit_summary(&self, mark: &QuitMark) -> Vec<String> {
        let Some(glue) = self.bridge.as_ref() else {
            return Vec::new();
        };
        let queued: Vec<RoomId> = self.pending_leaves.iter().cloned().collect();
        glue.quit_summary(mark, &self.state, &queued)
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    fn refresh_bridge_view(&mut self) {
        let Some(glue) = self.bridge.as_ref() else {
            return;
        };
        let view = BridgeView {
            line: status_line(glue, self.pending_leaves.len()),
            review_open: glue.editor().review().is_some(),
        };
        if self.state.bridge.as_ref() != Some(&view) {
            self.state.bridge = Some(view);
            self.state.update_actions(self.state.active_pane.clone());
        }
    }
}

fn selected_row_id(state: &super::state::AppState) -> Option<MessageId> {
    let room = state.rooms_list.active_room_id()?;
    let index = state.messages_list.selected_index()?;
    state
        .cache
        .rows_in_room(room)
        .nth(index)
        .map(|row| row.id().clone())
}

fn row_index(state: &super::state::AppState, id: &MessageId) -> Option<usize> {
    let room = state.rooms_list.active_room_id()?;
    state
        .cache
        .rows_in_room(room)
        .position(|row| row.id() == id)
}

fn refusal_text(refusal: WriteRefusal) -> &'static str {
    match refusal {
        WriteRefusal::Submission(SubmissionError::ReviewNeeded) => {
            "Review the previous failed or uncertain write first (x to dismiss)."
        }
        WriteRefusal::Submission(SubmissionError::Busy) | WriteRefusal::Busy => {
            "Another write is still pending; nothing was sent."
        }
        WriteRefusal::Submission(SubmissionError::QueueNotReady) | WriteRefusal::NotReady => {
            "The bridge session is not ready; nothing was sent."
        }
        WriteRefusal::Submission(SubmissionError::QueueClosed) | WriteRefusal::Closed => {
            "The bridge connection is closed; nothing was sent."
        }
        WriteRefusal::ThreadRootDeleted => {
            "The start of this thread was deleted; the reply was not sent."
        }
        WriteRefusal::Submission(_) | WriteRefusal::Invalid => {
            "This message cannot be sent as written; nothing was sent."
        }
    }
}

/// One line, metadata only: never a body, name or ID.
pub(crate) fn status_line(glue: &BridgeGlue, queued_leaves: usize) -> String {
    let status = glue.status();
    let mut parts: Vec<String> = Vec::new();
    if status.closed {
        parts.push("bridge disconnected: showing stale data, writes unavailable".into());
    } else {
        parts.push(
            match status.phase {
                Some(SessionPhase::Active) => "bridge online",
                Some(SessionPhase::Connecting | SessionPhase::AwaitingReady) | None => {
                    "bridge connecting"
                }
                Some(SessionPhase::SnapshotRequired) => "bridge resyncing",
                Some(SessionPhase::Expired | SessionPhase::Failed(_)) => "bridge reconnecting",
                Some(SessionPhase::Disconnected) => "bridge disconnected",
            }
            .into(),
        );
        match status.projection.upstream_webex {
            ConnectionState::Connected => parts.push("Webex connected".into()),
            ConnectionState::AuthRequired => {
                parts.push("Webex login required (mirror only; run webex-bridge login)".into())
            }
            ConnectionState::Disconnected => parts.push("Webex offline (mirror only)".into()),
            ConnectionState::Unknown | ConnectionState::Connecting => {}
        }
    }
    match glue.editor().status() {
        EditorWriteStatus::Pending => parts.push("sending…".into()),
        EditorWriteStatus::Rejected => parts.push("not sent: rejected (x to dismiss)".into()),
        EditorWriteStatus::DeliveryUnknown => {
            parts.push("delivery unknown: check the room before resending (x to dismiss)".into())
        }
        EditorWriteStatus::Unavailable => {
            parts.push("not sent: bridge unavailable (x to dismiss)".into())
        }
        EditorWriteStatus::Ready | EditorWriteStatus::Cancelled => {}
    }
    if let Some(outcome) = glue.last_action() {
        let (verb, action) = match outcome {
            ActionOutcome::Confirmed(action) => ("done", action),
            ActionOutcome::Rejected(action) => ("rejected", action),
            ActionOutcome::Unknown(action) => ("outcome unknown, rechecking", action),
            ActionOutcome::NotSent(action) => ("not sent", action),
        };
        let what = match action {
            GlueAction::DeleteMessage { .. } => "delete",
            GlueAction::LeaveRoom(_) => "leave",
        };
        parts.push(format!("{what} {verb}"));
    }
    if queued_leaves > 0 {
        parts.push(format!("{queued_leaves} leave(s) queued"));
    }
    if !status.failed_rooms.is_empty() {
        parts.push(format!(
            "{} room(s) failed to load (reopen to retry)",
            status.failed_rooms.len()
        ));
    }
    parts.join(" · ")
}
