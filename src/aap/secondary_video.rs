//! ARDA V27 — independent secondary AAP video channel bookkeeping.
//!
//! This is a source-only, non-transmitting stage. It does not create an
//! encoder, open any AAP channel, send a media frame, or alter MAIN. The
//! documented transitions are the contract for a future opt-in sender.
//! HU `display_id` is a *remote* display identifier, not an Android/SF ID.

use super::video_routes::{RoutePlan, VideoRole};

/// Production secondary transport is deliberately unavailable in V27.
pub const SECONDARY_TX_ENABLED: bool = false;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SecondaryPhase {
    Discovered,
    OpenRequested,
    Opened,
    Configured,
    Streaming,
    Suspended,
    Closed,
    Failed,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SecondarySlot {
    pub channel: u8,
    pub remote_display_id: u32,
    pub role: VideoRole,
    pub phase: SecondaryPhase,
    pub configuration_index: Option<usize>,
    pub media_session_id: Option<i32>,
    pub max_unacked: Option<u32>,
    pub last_ack: Option<u32>,
    pub ack_events: u64,
    pub observed_control: u64,
    pub observed_data: u64,
}

#[derive(Debug, Clone, Default)]
pub struct SecondaryVideoState {
    slots: Vec<SecondarySlot>,
}

#[allow(dead_code)]
impl SecondaryVideoState {
    pub fn from_plan(plan: &RoutePlan) -> Self {
        let slots = plan.routes.iter()
            .filter(|r| matches!(r.role, VideoRole::Cluster | VideoRole::Auxiliary))
            .map(|r| SecondarySlot {
                channel: r.service_id,
                remote_display_id: r.display_id,
                role: r.role,
                phase: SecondaryPhase::Discovered,
                configuration_index: r.selected_config_index,
                media_session_id: None,
                max_unacked: None,
                last_ack: None,
                ack_events: 0,
                observed_control: 0,
                observed_data: 0,
            }).collect();
        Self { slots }
    }

    pub fn slots(&self) -> &[SecondarySlot] { &self.slots }

    fn slot_mut(&mut self, channel: u8) -> Result<&mut SecondarySlot, String> {
        self.slots.iter_mut().find(|s| s.channel == channel)
            .ok_or_else(|| format!("secondary_channel_not_discovered:{channel}"))
    }

    /// Passive observation ONLY. This must never send a response or change
    /// session phase: an unsolicited frame is not proof the channel is open.
    pub fn observe(&mut self, channel: u8, is_control: bool) -> Option<String> {
        let slot = self.slots.iter_mut().find(|s| s.channel == channel)?;
        let counter = if is_control { &mut slot.observed_control } else { &mut slot.observed_data };
        *counter = counter.saturating_add(1);
        // Do not flood logs if a HU unexpectedly sends data on a closed channel.
        if *counter == 1 {
            Some(format!(
                "ARDA_V27_SECONDARY_PASSIVE role={:?} service_id={} remote_display_id={} control={} phase={:?} secondary_tx=DISABLED",
                slot.role, slot.channel, slot.remote_display_id, is_control, slot.phase,
            ))
        } else { None }
    }

    pub fn open_requested(&mut self, channel: u8) -> Result<(), String> {
        let slot = self.slot_mut(channel)?;
        if slot.phase != SecondaryPhase::Discovered {
            return Err(format!("invalid_open_request_phase:{:?}", slot.phase));
        }
        slot.phase = SecondaryPhase::OpenRequested;
        Ok(())
    }

    pub fn opened(&mut self, channel: u8) -> Result<(), String> {
        let slot = self.slot_mut(channel)?;
        if slot.phase != SecondaryPhase::OpenRequested {
            return Err(format!("unsolicited_open_response:{:?}", slot.phase));
        }
        slot.phase = SecondaryPhase::Opened;
        Ok(())
    }

    pub fn configured(&mut self, channel: u8, index: usize) -> Result<(), String> {
        let slot = self.slot_mut(channel)?;
        if slot.phase != SecondaryPhase::Opened {
            return Err(format!("invalid_config_phase:{:?}", slot.phase));
        }
        slot.configuration_index = Some(index);
        slot.phase = SecondaryPhase::Configured;
        Ok(())
    }

    pub fn start(&mut self, channel: u8, session: i32, max_unacked: u32) -> Result<(), String> {
        if session <= 0 || max_unacked == 0 {
            return Err("invalid_media_session_or_ack_window".into());
        }
        let slot = self.slot_mut(channel)?;
        if slot.phase != SecondaryPhase::Configured {
            return Err(format!("invalid_start_phase:{:?}", slot.phase));
        }
        slot.media_session_id = Some(session);
        slot.max_unacked = Some(max_unacked);
        slot.last_ack = None;
        slot.ack_events = 0;
        slot.phase = SecondaryPhase::Streaming;
        Ok(())
    }

    pub fn ack(&mut self, channel: u8, session: i32, ack: u32) -> Result<(), String> {
        let slot = self.slot_mut(channel)?;
        if slot.phase != SecondaryPhase::Streaming || slot.media_session_id != Some(session) {
            return Err("ack_wrong_phase_or_session".into());
        }
        // Some HU implementations emit a cumulative counter; only store an
        // observed value here. Sender-specific flow control will decide how
        // to interpret it in the actual TX integration.
        slot.last_ack = Some(ack);
        slot.ack_events = slot.ack_events.saturating_add(1);
        Ok(())
    }

    pub fn suspend(&mut self, channel: u8) -> Result<(), String> {
        let slot = self.slot_mut(channel)?;
        if slot.phase != SecondaryPhase::Streaming { return Err("invalid_suspend_phase".into()); }
        slot.phase = SecondaryPhase::Suspended;
        Ok(())
    }

    pub fn resume(&mut self, channel: u8) -> Result<(), String> {
        let slot = self.slot_mut(channel)?;
        if slot.phase != SecondaryPhase::Suspended { return Err("invalid_resume_phase".into()); }
        slot.phase = SecondaryPhase::Streaming;
        Ok(())
    }

    pub fn close(&mut self, channel: u8) -> Result<(), String> {
        let slot = self.slot_mut(channel)?;
        slot.phase = SecondaryPhase::Closed;
        slot.media_session_id = None;
        slot.max_unacked = None;
        Ok(())
    }

    pub fn summary_lines(&self) -> Vec<String> {
        self.slots.iter().map(|s| format!(
            "ARDA_V27_SECONDARY_SUMMARY service_id={} role={:?} remote_display_id={} phase={:?} config={:?} passive_control={} passive_data={} ack_events={} secondary_tx=DISABLED",
            s.channel, s.role, s.remote_display_id, s.phase,
            s.configuration_index, s.observed_control, s.observed_data, s.ack_events,
        )).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::proto::MediaSinkSelection;
    use super::super::video_routes::plan;

    fn sink(id: i32, role: &str, display: u32) -> MediaSinkSelection {
        MediaSinkSelection {
            service_id: id,
            available_type: "MEDIA_CODEC_VIDEO_H264_BP".into(),
            display_id: display,
            display_type: role.into(),
            video_configs: 0,
            first_video_config: None,
            video_config_selections: vec![],
            selected_video_config: None,
        }
    }

    fn sample() -> SecondaryVideoState {
        let routes = plan(&[
            sink(1, "DISPLAY_TYPE_MAIN", 0),
            sink(7, "DISPLAY_TYPE_CLUSTER", 1),
            sink(8, "DISPLAY_TYPE_AUXILIARY", 2),
            sink(9, "DISPLAY_TYPE_AUXILIARY", 3),
        ], Some(1)).unwrap();
        SecondaryVideoState::from_plan(&routes)
    }

    #[test]
    fn cluster_one_aux_many_main_excluded() {
        let state = sample();
        assert_eq!(state.slots().len(), 3);
        assert_eq!(state.slots()[0].channel, 7);
        assert_eq!(state.slots()[2].channel, 9);
        assert!(!SECONDARY_TX_ENABLED);
    }
    #[test]
    fn passive_does_not_open_channel() {
        let mut state = sample();
        assert!(state.observe(7, true).is_some());
        assert!(state.observe(7, true).is_none());
        assert!(state.observe(1, true).is_none());
        assert_eq!(state.slots()[0].phase, SecondaryPhase::Discovered);
    }
    #[test]
    fn open_requires_explicit_request() {
        let mut state = sample();
        assert!(state.opened(7).is_err());
        state.open_requested(7).unwrap();
        state.opened(7).unwrap();
        state.configured(7, 2).unwrap();
        state.start(7, 3, 2).unwrap();
        state.ack(7, 3, 1).unwrap();
        assert!(state.ack(7, 4, 1).is_err());
        state.suspend(7).unwrap();
        state.resume(7).unwrap();
        state.close(7).unwrap();
        assert_eq!(state.slots()[0].phase, SecondaryPhase::Closed);
    }
    #[test]
    fn channels_are_isolated() {
        let mut state = sample();
        state.open_requested(7).unwrap();
        assert_eq!(state.slots()[0].phase, SecondaryPhase::OpenRequested);
        assert_eq!(state.slots()[1].phase, SecondaryPhase::Discovered);
        assert!(state.open_requested(1).is_err());
        assert!(state.start(8, 10, 1).is_err());
        assert!(state.start(7, 0, 1).is_err());
    }
}
