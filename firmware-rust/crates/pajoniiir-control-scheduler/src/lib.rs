#![no_std]
#![forbid(unsafe_code)]

use pajoniiir_controller_core::{
    ControlEvent, ControlValue, DeckExtAction, DeckId, PadMode, SemanticControl,
};

pub const DISCRETE_FIFO_CAPACITY: usize = 32;
pub const CONTINUOUS_CAPACITY: usize = 32;
pub const HELD_SIMPLE_COUNT: usize = 6;
pub const HELD_PER_DECK_PAD_COUNT: usize = 24;
pub const HELD_STATE_COUNT: usize = HELD_SIMPLE_COUNT + 2 * HELD_PER_DECK_PAD_COUNT;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContinuousMode {
    LatestValue,
    AccumulateDelta,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SchedulerStats {
    pub fifo_full: u32,
    pub continuous_coalesced: u32,
    pub continuous_slot_full: u32,
    pub jog_saturated: u32,
    pub max_fifo_depth: u32,
}

#[derive(Clone, Copy)]
struct ContinuousSlot {
    event: Option<ControlEvent>,
    dirty: bool,
}

impl ContinuousSlot {
    const EMPTY: Self = Self {
        event: None,
        dirty: false,
    };
}

pub struct EventScheduler {
    fifo: [Option<ControlEvent>; DISCRETE_FIFO_CAPACITY],
    fifo_head: usize,
    fifo_tail: usize,
    fifo_count: usize,
    continuous: [ContinuousSlot; CONTINUOUS_CAPACITY],
    continuous_cursor: usize,
    stats: SchedulerStats,
}

impl Default for EventScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl EventScheduler {
    pub const fn new() -> Self {
        Self {
            fifo: [None; DISCRETE_FIFO_CAPACITY],
            fifo_head: 0,
            fifo_tail: 0,
            fifo_count: 0,
            continuous: [ContinuousSlot::EMPTY; CONTINUOUS_CAPACITY],
            continuous_cursor: 0,
            stats: SchedulerStats {
                fifo_full: 0,
                continuous_coalesced: 0,
                continuous_slot_full: 0,
                jog_saturated: 0,
                max_fifo_depth: 0,
            },
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn enqueue_discrete(&mut self, event: ControlEvent) -> bool {
        if self.fifo_count >= DISCRETE_FIFO_CAPACITY {
            self.stats.fifo_full = self.stats.fifo_full.saturating_add(1);
            return false;
        }

        self.fifo[self.fifo_tail] = Some(event);
        self.fifo_tail = (self.fifo_tail + 1) % DISCRETE_FIFO_CAPACITY;
        self.fifo_count += 1;
        self.stats.max_fifo_depth = self.stats.max_fifo_depth.max(self.fifo_count as u32);
        true
    }

    pub fn dequeue_discrete(&mut self) -> Option<ControlEvent> {
        if self.fifo_count == 0 {
            return None;
        }

        let event = self.fifo[self.fifo_head].take();
        self.fifo_head = (self.fifo_head + 1) % DISCRETE_FIFO_CAPACITY;
        self.fifo_count -= 1;
        event
    }

    pub fn publish_continuous(&mut self, event: ControlEvent, mode: ContinuousMode) -> bool {
        let Some(index) = self.find_continuous_slot(event) else {
            self.stats.continuous_slot_full = self.stats.continuous_slot_full.saturating_add(1);
            return false;
        };

        let slot = &mut self.continuous[index];
        if slot.event.is_none() || !slot.dirty {
            slot.event = Some(event);
            slot.dirty = true;
            return true;
        }

        self.stats.continuous_coalesced = self.stats.continuous_coalesced.saturating_add(1);

        match mode {
            ContinuousMode::LatestValue => {
                slot.event = Some(event);
            }
            ContinuousMode::AccumulateDelta => {
                let Some(existing) = slot.event else {
                    return false;
                };
                let (ControlValue::Relative(current), ControlValue::Relative(next)) =
                    (existing.value, event.value)
                else {
                    return false;
                };

                let sum = current as i32 + next as i32;
                let saturated = sum.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                if sum != saturated as i32 {
                    self.stats.jog_saturated = self.stats.jog_saturated.saturating_add(1);
                }
                slot.event = Some(ControlEvent {
                    value: ControlValue::Relative(saturated),
                    ..event
                });
            }
        }

        slot.dirty = true;
        true
    }

    pub fn take_continuous(&mut self) -> Option<ControlEvent> {
        for offset in 0..CONTINUOUS_CAPACITY {
            let index = (self.continuous_cursor + offset) % CONTINUOUS_CAPACITY;
            let slot = &mut self.continuous[index];
            if !slot.dirty {
                continue;
            }
            let Some(event) = slot.event else {
                continue;
            };

            slot.dirty = false;
            self.continuous_cursor = (index + 1) % CONTINUOUS_CAPACITY;
            return Some(event);
        }

        None
    }

    pub const fn stats(&self) -> SchedulerStats {
        self.stats
    }

    fn find_continuous_slot(&self, event: ControlEvent) -> Option<usize> {
        let mut free = None;

        for (index, slot) in self.continuous.iter().enumerate() {
            if let Some(existing) = slot.event {
                if existing.deck == event.deck && existing.control == event.control {
                    return Some(index);
                }
            } else if free.is_none() {
                free = Some(index);
            }
        }

        free
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirtyHeldState {
    pub key: usize,
    pub event: ControlEvent,
    pub sequence: u8,
}

#[derive(Clone, Copy)]
struct HeldSlot {
    desired: Option<ControlEvent>,
    scheduled_value: Option<ControlValue>,
    dirty: bool,
    sequence: u8,
}

impl HeldSlot {
    const EMPTY: Self = Self {
        desired: None,
        scheduled_value: None,
        dirty: false,
        sequence: 0,
    };
}

pub struct HeldStateReconciler {
    slots: [HeldSlot; HELD_STATE_COUNT],
}

impl Default for HeldStateReconciler {
    fn default() -> Self {
        Self::new()
    }
}

impl HeldStateReconciler {
    pub const fn new() -> Self {
        Self {
            slots: [HeldSlot::EMPTY; HELD_STATE_COUNT],
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn key(event: ControlEvent) -> Option<usize> {
        match (event.deck, event.control, event.value) {
            (Some(DeckId::One), SemanticControl::JogTouch, ControlValue::Pressed(_)) => Some(0),
            (Some(DeckId::Two), SemanticControl::JogTouch, ControlValue::Pressed(_)) => Some(1),
            (Some(DeckId::One), SemanticControl::Shift, ControlValue::Pressed(_)) => Some(2),
            (Some(DeckId::Two), SemanticControl::Shift, ControlValue::Pressed(_)) => Some(3),
            (
                Some(DeckId::One),
                SemanticControl::DeckExtAction,
                ControlValue::DeckExtAction(action),
            ) if action.action == DeckExtAction::Censor => Some(4),
            (
                Some(DeckId::Two),
                SemanticControl::DeckExtAction,
                ControlValue::DeckExtAction(action),
            ) if action.action == DeckExtAction::Censor => Some(5),
            (Some(deck), SemanticControl::PadAction, ControlValue::PadAction(pad))
                if pad.pad < 8 =>
            {
                let mode_offset = match (pad.mode, pad.shifted) {
                    (PadMode::PadFx1, _) => 0,
                    (PadMode::PadFx2, _) => 8,
                    (PadMode::BeatLoop, true) => 16,
                    _ => return None,
                };
                let deck_offset = match deck {
                    DeckId::One => 0,
                    DeckId::Two => HELD_PER_DECK_PAD_COUNT,
                };
                Some(HELD_SIMPLE_COUNT + deck_offset + mode_offset + pad.pad as usize)
            }
            _ => None,
        }
    }

    pub fn observe(&mut self, event: ControlEvent, sequence: u8) -> Option<usize> {
        let key = Self::key(event)?;
        let slot = &mut self.slots[key];
        slot.desired = Some(event);
        slot.sequence = sequence;
        slot.dirty = slot.scheduled_value != Some(event.value);
        Some(key)
    }

    pub fn mark_scheduled(&mut self, key: usize, value: ControlValue) -> bool {
        let Some(slot) = self.slots.get_mut(key) else {
            return false;
        };
        let Some(desired) = slot.desired else {
            return false;
        };

        slot.scheduled_value = Some(value);
        slot.dirty = desired.value != value;
        true
    }

    pub fn next_dirty(&self, cursor: &mut usize) -> Option<DirtyHeldState> {
        while *cursor < HELD_STATE_COUNT {
            let key = *cursor;
            *cursor += 1;
            let slot = self.slots[key];
            if !slot.dirty {
                continue;
            }
            let event = slot.desired?;
            return Some(DirtyHeldState {
                key,
                event,
                sequence: slot.sequence,
            });
        }

        None
    }

    pub fn invalidate_scheduled(&mut self) {
        for slot in &mut self.slots {
            if slot.desired.is_none() {
                continue;
            }
            slot.scheduled_value = None;
            slot.dirty = true;
        }
    }

    pub fn release_all(&mut self, sequence: u8) {
        for slot in &mut self.slots {
            let Some(event) = slot.desired else {
                continue;
            };
            let released = release_event(event);
            slot.desired = Some(released);
            slot.sequence = sequence;
            slot.dirty = slot.scheduled_value != Some(released.value);
        }
    }
}

fn release_event(event: ControlEvent) -> ControlEvent {
    let value = match event.value {
        ControlValue::Pressed(_) => ControlValue::Pressed(false),
        ControlValue::DeckExtAction(mut action) => {
            action.pressed = false;
            ControlValue::DeckExtAction(action)
        }
        ControlValue::PadAction(mut pad) => {
            pad.pressed = false;
            ControlValue::PadAction(pad)
        }
        value => value,
    };

    ControlEvent { value, ..event }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionState {
    Connected,
    Disconnected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectionDelivery {
    pub state: ConnectionState,
    generation: u32,
    disconnect_generation: Option<u32>,
    captured_connected: bool,
}

pub struct ConnectionReconciler {
    connected: bool,
    generation: u32,
    acknowledged_generation: u32,
    disconnect_pending: bool,
    disconnect_generation: u32,
    snapshot_pending: bool,
}

impl Default for ConnectionReconciler {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectionReconciler {
    pub const fn new() -> Self {
        Self {
            connected: false,
            generation: 0,
            acknowledged_generation: 0,
            disconnect_pending: false,
            disconnect_generation: 0,
            snapshot_pending: false,
        }
    }

    pub const fn connected(&self) -> bool {
        self.connected
    }

    pub const fn generation(&self) -> u32 {
        self.generation
    }

    pub fn set_connected(
        &mut self,
        connected: bool,
        held: &mut HeldStateReconciler,
        release_sequence: u8,
    ) {
        if connected == self.connected {
            return;
        }

        self.connected = connected;
        self.generation = self.generation.wrapping_add(1);

        if connected {
            held.invalidate_scheduled();
            self.snapshot_pending = true;
        } else {
            self.disconnect_pending = true;
            self.disconnect_generation = self.generation;
            self.snapshot_pending = false;
            held.release_all(release_sequence);
        }
    }

    pub fn pending_delivery(&self) -> Option<ConnectionDelivery> {
        if self.disconnect_pending {
            return Some(ConnectionDelivery {
                state: ConnectionState::Disconnected,
                generation: self.generation,
                disconnect_generation: Some(self.disconnect_generation),
                captured_connected: self.connected,
            });
        }

        if self.generation == self.acknowledged_generation {
            return None;
        }

        Some(ConnectionDelivery {
            state: if self.connected {
                ConnectionState::Connected
            } else {
                ConnectionState::Disconnected
            },
            generation: self.generation,
            disconnect_generation: None,
            captured_connected: self.connected,
        })
    }

    pub fn mark_delivered(&mut self, delivery: ConnectionDelivery) {
        if let Some(disconnect_generation) = delivery.disconnect_generation {
            if self.disconnect_pending && self.disconnect_generation == disconnect_generation {
                self.disconnect_pending = false;
            }
            if !delivery.captured_connected {
                self.acknowledged_generation = delivery.generation;
            }
        } else {
            self.acknowledged_generation = delivery.generation;
        }
    }

    pub fn take_snapshot_request(&mut self) -> bool {
        if !self.connected || !self.snapshot_pending {
            return false;
        }
        self.snapshot_pending = false;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pajoniiir_controller_core::{DeckExtActionValue, PadAction};

    fn pressed(deck: DeckId, control: SemanticControl, down: bool) -> ControlEvent {
        ControlEvent {
            deck: Some(deck),
            control,
            value: ControlValue::Pressed(down),
        }
    }

    fn relative(deck: DeckId, control: SemanticControl, value: i16) -> ControlEvent {
        ControlEvent {
            deck: Some(deck),
            control,
            value: ControlValue::Relative(value),
        }
    }

    fn crossfader(value: u16) -> ControlEvent {
        ControlEvent {
            deck: None,
            control: SemanticControl::Crossfader,
            value: ControlValue::Absolute { value, max: 0x3fff },
        }
    }

    fn censor(deck: DeckId, down: bool) -> ControlEvent {
        ControlEvent {
            deck: Some(deck),
            control: SemanticControl::DeckExtAction,
            value: ControlValue::DeckExtAction(DeckExtActionValue {
                action: DeckExtAction::Censor,
                pressed: down,
            }),
        }
    }

    fn pad(deck: DeckId, mode: PadMode, index: u8, shifted: bool, down: bool) -> ControlEvent {
        ControlEvent {
            deck: Some(deck),
            control: SemanticControl::PadAction,
            value: ControlValue::PadAction(PadAction {
                pad: index,
                mode,
                shifted,
                pressed: down,
            }),
        }
    }

    #[test]
    fn discrete_fifo_and_continuous_state_are_independent() {
        let mut scheduler = EventScheduler::new();
        let first = pressed(DeckId::One, SemanticControl::Play, true);
        let second = pressed(DeckId::One, SemanticControl::Cue, true);

        assert!(scheduler.enqueue_discrete(first));
        assert!(scheduler.enqueue_discrete(second));
        assert_eq!(scheduler.dequeue_discrete(), Some(first));
        assert!(scheduler.publish_continuous(crossfader(7000), ContinuousMode::LatestValue));
        assert_eq!(scheduler.dequeue_discrete(), Some(second));
        assert_eq!(scheduler.dequeue_discrete(), None);
        assert_eq!(scheduler.take_continuous(), Some(crossfader(7000)));
    }

    #[test]
    fn discrete_fifo_is_bounded_and_measured() {
        let mut scheduler = EventScheduler::new();
        for _ in 0..DISCRETE_FIFO_CAPACITY {
            assert!(scheduler.enqueue_discrete(pressed(DeckId::One, SemanticControl::Cue, true)));
        }
        assert!(!scheduler.enqueue_discrete(pressed(DeckId::Two, SemanticControl::Play, true)));
        assert_eq!(scheduler.stats().fifo_full, 1);
        assert_eq!(
            scheduler.stats().max_fifo_depth,
            DISCRETE_FIFO_CAPACITY as u32
        );
    }

    #[test]
    fn latest_value_and_jog_accumulation_match_released_behavior() {
        let mut scheduler = EventScheduler::new();
        assert!(scheduler.publish_continuous(crossfader(10), ContinuousMode::LatestValue));
        assert!(scheduler.publish_continuous(crossfader(20), ContinuousMode::LatestValue));
        assert!(scheduler.publish_continuous(crossfader(30), ContinuousMode::LatestValue));

        assert!(scheduler.publish_continuous(
            relative(DeckId::One, SemanticControl::JogBend, 32760),
            ContinuousMode::AccumulateDelta
        ));
        assert!(scheduler.publish_continuous(
            relative(DeckId::One, SemanticControl::JogBend, 100),
            ContinuousMode::AccumulateDelta
        ));

        assert_eq!(scheduler.take_continuous(), Some(crossfader(30)));
        assert_eq!(
            scheduler.take_continuous(),
            Some(relative(DeckId::One, SemanticControl::JogBend, i16::MAX))
        );
        assert_eq!(scheduler.take_continuous(), None);

        assert!(scheduler.publish_continuous(
            relative(DeckId::One, SemanticControl::JogBend, -7),
            ContinuousMode::AccumulateDelta
        ));
        assert_eq!(
            scheduler.take_continuous(),
            Some(relative(DeckId::One, SemanticControl::JogBend, -7))
        );
        assert_eq!(scheduler.stats().continuous_coalesced, 3);
        assert_eq!(scheduler.stats().jog_saturated, 1);
    }

    #[test]
    fn held_keys_are_precise_and_do_not_collapse_commands() {
        let touch = HeldStateReconciler::key(pressed(DeckId::One, SemanticControl::JogTouch, true));
        let shift = HeldStateReconciler::key(pressed(DeckId::One, SemanticControl::Shift, true));
        let censor_key = HeldStateReconciler::key(censor(DeckId::One, true));
        let pad_fx1 = HeldStateReconciler::key(pad(DeckId::One, PadMode::PadFx1, 3, false, true));
        let pad_fx2 = HeldStateReconciler::key(pad(DeckId::One, PadMode::PadFx2, 3, false, true));
        let roll = HeldStateReconciler::key(pad(DeckId::Two, PadMode::BeatLoop, 3, true, true));

        assert!(touch.is_some());
        assert_ne!(shift, touch);
        assert_ne!(censor_key, shift);
        assert_ne!(pad_fx1, censor_key);
        assert_ne!(pad_fx2, pad_fx1);
        assert_ne!(roll, pad_fx2);
        assert_eq!(
            HeldStateReconciler::key(pressed(DeckId::One, SemanticControl::Play, true)),
            None
        );
        assert_eq!(
            HeldStateReconciler::key(pad(DeckId::One, PadMode::HotCue, 3, false, true)),
            None
        );
    }

    #[test]
    fn latest_physical_level_wins_before_delivery() {
        let mut state = HeldStateReconciler::new();
        let down = pressed(DeckId::One, SemanticControl::JogTouch, true);
        let up = pressed(DeckId::One, SemanticControl::JogTouch, false);

        let key = state.observe(down, 10).unwrap();
        assert!(state.mark_scheduled(key, down.value));
        assert!(state.observe(up, 11).is_some());
        assert!(state.observe(down, 12).is_some());

        let mut cursor = 0;
        assert_eq!(state.next_dirty(&mut cursor), None);

        assert!(state.observe(up, 13).is_some());
        let mut cursor = 0;
        let dirty = state.next_dirty(&mut cursor).unwrap();
        assert_eq!(dirty.key, key);
        assert_eq!(dirty.event, up);
        assert_eq!(dirty.sequence, 13);
    }

    #[test]
    fn stale_scheduled_snapshot_keeps_newer_level_dirty() {
        let mut state = HeldStateReconciler::new();
        let down = pressed(DeckId::One, SemanticControl::Shift, true);
        let up = pressed(DeckId::One, SemanticControl::Shift, false);
        let key = state.observe(down, 1).unwrap();
        state.observe(up, 2).unwrap();
        assert!(state.mark_scheduled(key, down.value));

        let mut cursor = 0;
        let dirty = state.next_dirty(&mut cursor).unwrap();
        assert_eq!(dirty.event, up);
    }

    #[test]
    fn disconnect_releases_all_observed_held_shapes() {
        let mut state = HeldStateReconciler::new();
        let shift = pressed(DeckId::Two, SemanticControl::Shift, true);
        let ext = censor(DeckId::Two, true);
        let pad_event = pad(DeckId::Two, PadMode::PadFx2, 6, false, true);

        let shift_key = state.observe(shift, 1).unwrap();
        let ext_key = state.observe(ext, 2).unwrap();
        let pad_key = state.observe(pad_event, 3).unwrap();
        state.mark_scheduled(shift_key, shift.value);
        state.mark_scheduled(ext_key, ext.value);
        state.mark_scheduled(pad_key, pad_event.value);

        state.release_all(44);

        let mut cursor = 0;
        let mut seen = 0;
        while let Some(dirty) = state.next_dirty(&mut cursor) {
            assert_eq!(dirty.sequence, 44);
            match dirty.event.value {
                ControlValue::Pressed(value) => assert!(!value),
                ControlValue::DeckExtAction(value) => assert!(!value.pressed),
                ControlValue::PadAction(value) => assert!(!value.pressed),
                _ => panic!("unexpected held value"),
            }
            seen += 1;
        }
        assert_eq!(seen, 3);
    }

    #[test]
    fn held_release_remains_durable_when_fifo_is_full() {
        let mut scheduler = EventScheduler::new();
        for _ in 0..DISCRETE_FIFO_CAPACITY {
            assert!(scheduler.enqueue_discrete(pressed(DeckId::One, SemanticControl::Cue, true)));
        }

        let mut held = HeldStateReconciler::new();
        let down = pressed(DeckId::One, SemanticControl::JogTouch, true);
        let up = pressed(DeckId::One, SemanticControl::JogTouch, false);
        let key = held.observe(down, 1).unwrap();
        held.mark_scheduled(key, down.value);
        held.observe(up, 2).unwrap();

        assert!(!scheduler.enqueue_discrete(up));

        let mut cursor = 0;
        let dirty = held.next_dirty(&mut cursor).unwrap();
        assert_eq!(dirty.event, up);
        assert_eq!(dirty.sequence, 2);
    }

    #[test]
    fn reconnect_cannot_erase_an_undelivered_disconnect_edge() {
        let mut held = HeldStateReconciler::new();
        let mut connection = ConnectionReconciler::new();

        connection.set_connected(true, &mut held, 0);
        let first = connection.pending_delivery().unwrap();
        assert_eq!(first.state, ConnectionState::Connected);
        connection.mark_delivered(first);

        connection.set_connected(false, &mut held, 1);
        connection.set_connected(true, &mut held, 2);

        let disconnect = connection.pending_delivery().unwrap();
        assert_eq!(disconnect.state, ConnectionState::Disconnected);
        connection.mark_delivered(disconnect);

        let reconnect = connection.pending_delivery().unwrap();
        assert_eq!(reconnect.state, ConnectionState::Connected);
        connection.mark_delivered(reconnect);
        assert_eq!(connection.pending_delivery(), None);
    }

    #[test]
    fn newer_disconnect_survives_delivery_of_an_older_connected_generation() {
        let mut held = HeldStateReconciler::new();
        let mut connection = ConnectionReconciler::new();

        connection.set_connected(true, &mut held, 0);
        let connected = connection.pending_delivery().unwrap();

        connection.set_connected(false, &mut held, 1);
        connection.mark_delivered(connected);

        let disconnect = connection.pending_delivery().unwrap();
        assert_eq!(disconnect.state, ConnectionState::Disconnected);
        connection.mark_delivered(disconnect);
        assert_eq!(connection.pending_delivery(), None);
        assert!(!connection.connected());
    }

    #[test]
    fn reconnect_invalidates_held_schedule_and_requests_one_snapshot() {
        let mut held = HeldStateReconciler::new();
        let down = pressed(DeckId::One, SemanticControl::JogTouch, true);
        let key = held.observe(down, 7).unwrap();
        assert!(held.mark_scheduled(key, down.value));

        let mut cursor = 0;
        assert_eq!(held.next_dirty(&mut cursor), None);

        let mut connection = ConnectionReconciler::new();
        connection.set_connected(true, &mut held, 0);

        let mut cursor = 0;
        assert_eq!(held.next_dirty(&mut cursor).unwrap().event, down);
        assert!(connection.take_snapshot_request());
        assert!(!connection.take_snapshot_request());
    }

    #[test]
    fn disconnect_releases_held_state_and_cancels_pending_snapshot() {
        let mut held = HeldStateReconciler::new();
        let down = pressed(DeckId::Two, SemanticControl::Shift, true);
        let key = held.observe(down, 1).unwrap();
        assert!(held.mark_scheduled(key, down.value));

        let mut connection = ConnectionReconciler::new();
        connection.set_connected(true, &mut held, 0);
        connection.set_connected(false, &mut held, 44);

        assert!(!connection.take_snapshot_request());
        let mut cursor = 0;
        let dirty = held.next_dirty(&mut cursor).unwrap();
        assert_eq!(dirty.sequence, 44);
        assert_eq!(dirty.event.value, ControlValue::Pressed(false));
    }
}
