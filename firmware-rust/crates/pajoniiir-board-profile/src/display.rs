//! HAL-independent Guition adapter contracts. No panel table/payload is bundled
//! and no firmware task calls these ports yet. Source candidates are not drivers.
use crate::{BoardId, BoardProfile, PanelController, TouchController};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DsiCandidate {
    pub pixel_clock_mhz: u16,
    pub lane_rate_mbps: Option<u16>,
    /// pulse / back porch / front porch
    pub horizontal: [u16; 3],
    pub vertical: [u16; 3],
    pub source_id: &'static str,
    pub source_sha256: &'static str,
    pub lane_source_sha256: &'static str,
}
pub const fn candidate(board: BoardId) -> DsiCandidate {
    match board {
        BoardId::GuitionJc1060P470cIwY => DsiCandidate {
            pixel_clock_mhz: 52,
            lane_rate_mbps: Some(750),
            horizontal: [24, 136, 160],
            vertical: [2, 21, 12],
            source_id: "jc1060-v3-bsp.c",
            source_sha256: "876fe2ba4b881ce90b2d01a9af1b44cefbd913a97312f30d57c54801135fc89f",
            lane_source_sha256: "876fe2ba4b881ce90b2d01a9af1b44cefbd913a97312f30d57c54801135fc89f",
        },
        BoardId::GuitionJc8012P4a1cIwY => DsiCandidate {
            pixel_clock_mhz: 70,
            lane_rate_mbps: Some(1000),
            horizontal: [20, 20, 40],
            vertical: [4, 10, 30],
            source_id: "jc8012-v3-bsp.c",
            source_sha256: "9dd83ba0c023c7ace470cb94a33b0110da7040a36d56049e9cd5c53a23c14ff0",
            lane_source_sha256: "c52d268608a3083f864b9790bf9d93198af33bf4a1790fb18458d52feaf1c9ce",
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TouchPins {
    /// Source conflict: schematic 22/21 versus BSP NC/NC. Never drive by default.
    Unresolved,
    Candidate {
        reset: u8,
        interrupt: u8,
    },
}
pub const fn touch_pins(board: BoardId) -> TouchPins {
    match board {
        BoardId::GuitionJc1060P470cIwY => TouchPins::Unresolved,
        BoardId::GuitionJc8012P4a1cIwY => TouchPins::Candidate {
            reset: 22,
            interrupt: 21,
        },
    }
}

/// Independently implementable physical ports. Callers must share I2C7/8 via a
/// single bus owner and bind exact board GPIOs; implementations are still pending.
pub trait PanelPorts {
    type Error;
    fn backlight_off(&mut self) -> Result<(), Self::Error>;
    fn phy_power(&mut self, enabled: bool, ldo: u8, millivolts: u16) -> Result<(), Self::Error>;
    fn reset_asserted(&mut self, asserted: bool) -> Result<(), Self::Error>;
    fn delay_ms(&mut self, millis: u16);
    fn configure_dsi(
        &mut self,
        profile: &BoardProfile,
        timing: DsiCandidate,
    ) -> Result<(), Self::Error>;
    fn command(&mut self, command: u8, data: &[u8]) -> Result<(), Self::Error>;
    fn start_scanout(&mut self) -> Result<(), Self::Error>;
    fn stop_scanout(&mut self) -> Result<(), Self::Error>;
    fn backlight_on(&mut self) -> Result<(), Self::Error>;
}

pub struct Command<'a> {
    pub command: u8,
    pub data: &'a [u8],
    pub delay_ms: u16,
}
/// External, reviewed selection for one exact unit. These fields document the
/// caller's evidence; they are not a license determination or hardware proof.
pub struct PanelSelection<'a> {
    pub board: BoardId,
    pub controller: PanelController,
    pub unit: &'a str,
    pub panel_marking: &'a str,
    pub table_sha256: &'a str,
    pub distribution_authorized: bool,
    pub wiring_confirmed: bool,
    pub lane_rate_mbps: u16,
    pub reset_assert_ms: u16,
    pub reset_release_ms: u16,
    pub commands: &'a [Command<'a>],
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Blocked {
    WrongBoard,
    MissingUnit,
    MissingPanel,
    MissingTable,
    Distribution,
    Wiring,
    Timing,
}
#[derive(Debug, Eq, PartialEq)]
pub enum Error<E> {
    Blocked(Blocked),
    Port { error: E, cleanup_failed: bool },
    AlreadyStarted,
    CleanupRequired,
}

/// Bounded sequencing boundary, not an activated DSI implementation. Backlight
/// is enabled only after an explicit table, reset and scanout. Failed operations
/// attempt all cleanup ports; failure of cleanup is retained, never hidden.
pub struct PanelAdapter<'a, P> {
    profile: &'a BoardProfile,
    ports: P,
    started: bool,
    cleanup_required: bool,
}
impl<'a, P: PanelPorts> PanelAdapter<'a, P> {
    pub fn new(profile: &'a BoardProfile, ports: P) -> Self {
        Self {
            profile,
            ports,
            started: false,
            cleanup_required: false,
        }
    }
    pub fn start(&mut self, selected: &PanelSelection<'_>) -> Result<(), Error<P::Error>> {
        if self.started {
            return Err(Error::AlreadyStarted);
        }
        if self.cleanup_required {
            return Err(Error::CleanupRequired);
        }
        validate(self.profile, selected).map_err(Error::Blocked)?;
        let result = self.start_ports(selected);
        match result {
            Ok(()) => {
                self.started = true;
                Ok(())
            }
            Err(error) => Err(Error::Port {
                error,
                cleanup_failed: self.shutdown(),
            }),
        }
    }
    fn start_ports(&mut self, selected: &PanelSelection<'_>) -> Result<(), P::Error> {
        let profile = self.profile;
        self.ports.backlight_off()?;
        self.ports.phy_power(
            true,
            profile.display.phy_ldo_channel,
            profile.display.phy_voltage_mv,
        )?;
        self.ports.reset_asserted(true)?;
        self.ports.delay_ms(selected.reset_assert_ms);
        self.ports.reset_asserted(false)?;
        self.ports.delay_ms(selected.reset_release_ms);
        let mut timing = candidate(profile.id);
        timing.lane_rate_mbps = Some(selected.lane_rate_mbps);
        self.ports.configure_dsi(profile, timing)?;
        for command in selected.commands {
            self.ports.command(command.command, command.data)?;
            self.ports.delay_ms(command.delay_ms);
        }
        self.ports.start_scanout()?;
        self.ports.backlight_on()
    }
    /// Returns true if any cleanup failed. Always attempts all four operations.
    pub fn shutdown(&mut self) -> bool {
        let profile = self.profile;
        let mut failed = self.ports.backlight_off().is_err();
        failed |= self.ports.stop_scanout().is_err();
        failed |= self.ports.reset_asserted(true).is_err();
        failed |= self
            .ports
            .phy_power(
                false,
                profile.display.phy_ldo_channel,
                profile.display.phy_voltage_mv,
            )
            .is_err();
        self.started = false;
        self.cleanup_required = failed;
        failed
    }
}
fn validate(profile: &BoardProfile, selected: &PanelSelection<'_>) -> Result<(), Blocked> {
    if selected.board != profile.id || selected.controller != profile.display.controller {
        return Err(Blocked::WrongBoard);
    }
    if selected.unit.is_empty() {
        return Err(Blocked::MissingUnit);
    }
    if selected.panel_marking.is_empty() {
        return Err(Blocked::MissingPanel);
    }
    if selected.table_sha256.len() != 64
        || !selected.table_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || selected.commands.is_empty()
        || selected.commands.len() > 512
        || selected
            .commands
            .iter()
            .any(|cmd| cmd.data.len() > 64 || cmd.delay_ms > 1000)
    {
        return Err(Blocked::MissingTable);
    }
    if !selected.distribution_authorized {
        return Err(Blocked::Distribution);
    }
    if !selected.wiring_confirmed {
        return Err(Blocked::Wiring);
    }
    if selected.lane_rate_mbps == 0
        || selected.reset_assert_ms == 0
        || selected.reset_assert_ms > 1000
        || selected.reset_release_ms == 0
        || selected.reset_release_ms > 1000
    {
        return Err(Blocked::Timing);
    }
    Ok(())
}

/// Touch has separate ownership/activation from the panel and backlight. Payload
/// provisioning is explicit; no vendor GSL3680 payload is included or inferred.
pub struct TouchSelection<'a> {
    pub board: BoardId,
    pub controller: TouchController,
    pub address: u8,
    pub reset_gpio: Option<u8>,
    pub interrupt_gpio: Option<u8>,
    pub wiring_confirmed: bool,
    pub coordinate_mapping_confirmed: bool,
    pub payload: Option<&'a [u8]>,
    pub payload_sha256: Option<&'a str>,
    pub distribution_authorized: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TouchBlocked {
    WrongBoard,
    Wiring,
    Address,
    Coordinates,
    Payload,
}
impl TouchSelection<'_> {
    pub fn validate(&self, profile: &BoardProfile) -> Result<(), TouchBlocked> {
        if self.board != profile.id || self.controller != profile.display.touch {
            return Err(TouchBlocked::WrongBoard);
        }
        if !self.wiring_confirmed
            || self.reset_gpio == self.interrupt_gpio && self.reset_gpio.is_some()
            || self.reset_gpio.is_some_and(|p| p > 54)
            || self.interrupt_gpio.is_some_and(|p| p > 54)
        {
            return Err(TouchBlocked::Wiring);
        }
        if !match self.controller {
            TouchController::Gt911 => matches!(self.address, 0x14 | 0x5d),
            TouchController::Gsl3680 => self.address == 0x40,
        } {
            return Err(TouchBlocked::Address);
        }
        if !self.coordinate_mapping_confirmed {
            return Err(TouchBlocked::Coordinates);
        }
        if profile.display.requires_touch_firmware
            && (self.payload.is_none_or(|p| p.is_empty())
                || self
                    .payload_sha256
                    .is_none_or(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
                || !self.distribution_authorized)
        {
            return Err(TouchBlocked::Payload);
        }
        Ok(())
    }
}
pub trait TouchPort {
    type Error;
    /// The exact reset/strap sequence belongs to the unit's physical adapter.
    fn initialize(&mut self, selection: &TouchSelection<'_>) -> Result<(), Self::Error>;
    /// Raw coordinates only; calibrated mapping/orientation belongs to the UI.
    fn read_point(&mut self) -> Result<Option<(u16, u16)>, Self::Error>;
    fn stop(&mut self) -> Result<(), Self::Error>;
}

#[derive(Debug, Eq, PartialEq)]
pub enum TouchError<E> {
    Blocked(TouchBlocked),
    Port(E),
    NotStarted,
    AlreadyStarted,
    CleanupRequired,
}
pub struct TouchAdapter<'a, P> {
    profile: &'a BoardProfile,
    ports: P,
    started: bool,
    cleanup_required: bool,
}
impl<'a, P: TouchPort> TouchAdapter<'a, P> {
    pub fn new(profile: &'a BoardProfile, ports: P) -> Self {
        Self {
            profile,
            ports,
            started: false,
            cleanup_required: false,
        }
    }
    pub fn start(&mut self, selection: &TouchSelection<'_>) -> Result<(), TouchError<P::Error>> {
        if self.cleanup_required {
            return Err(TouchError::CleanupRequired);
        }
        if self.started {
            return Err(TouchError::AlreadyStarted);
        }
        selection
            .validate(self.profile)
            .map_err(TouchError::Blocked)?;
        if let Err(error) = self.ports.initialize(selection) {
            self.cleanup_required = self.ports.stop().is_err();
            return Err(TouchError::Port(error));
        }
        self.started = true;
        Ok(())
    }
    pub fn read_point(&mut self) -> Result<Option<(u16, u16)>, TouchError<P::Error>> {
        if !self.started {
            return Err(TouchError::NotStarted);
        }
        self.ports.read_point().map_err(TouchError::Port)
    }
    pub fn stop(&mut self) -> Result<(), TouchError<P::Error>> {
        self.started = false;
        self.cleanup_required = true;
        self.ports.stop().map_err(TouchError::Port)?;
        self.cleanup_required = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;
    struct Ports {
        calls: Vec<&'static str>,
        fail: Option<&'static str>,
        cleanup_fail: bool,
    }
    impl Ports {
        fn step(&mut self, name: &'static str) -> Result<(), u8> {
            self.calls.push(name);
            if self.fail == Some(name) || self.cleanup_fail && name == "stop" {
                Err(1)
            } else {
                Ok(())
            }
        }
    }
    impl PanelPorts for Ports {
        type Error = u8;
        fn backlight_off(&mut self) -> Result<(), u8> {
            self.step("dark")
        }
        fn phy_power(&mut self, on: bool, _: u8, _: u16) -> Result<(), u8> {
            self.step(if on { "power" } else { "off" })
        }
        fn reset_asserted(&mut self, on: bool) -> Result<(), u8> {
            self.step(if on { "reset" } else { "release" })
        }
        fn delay_ms(&mut self, _: u16) {
            self.calls.push("delay");
        }
        fn configure_dsi(&mut self, _: &BoardProfile, _: DsiCandidate) -> Result<(), u8> {
            self.step("dsi")
        }
        fn command(&mut self, _: u8, _: &[u8]) -> Result<(), u8> {
            self.step("command")
        }
        fn start_scanout(&mut self) -> Result<(), u8> {
            self.step("scan")
        }
        fn stop_scanout(&mut self) -> Result<(), u8> {
            self.step("stop")
        }
        fn backlight_on(&mut self) -> Result<(), u8> {
            self.step("light")
        }
    }
    const COMMANDS: [Command<'static>; 1] = [Command {
        command: 0x11,
        data: &[],
        delay_ms: 120,
    }];
    fn selection() -> PanelSelection<'static> {
        PanelSelection {
            board: BoardId::GuitionJc1060P470cIwY,
            controller: PanelController::Jd9165,
            unit: "TEST-ONLY",
            panel_marking: "FIXTURE",
            table_sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            distribution_authorized: true,
            wiring_confirmed: true,
            lane_rate_mbps: 750,
            reset_assert_ms: 10,
            reset_release_ms: 120,
            commands: &COMMANDS,
        }
    }
    #[test]
    fn blocked_selection_touches_no_ports_and_failure_attempts_all_cleanup() {
        let profile = &crate::jc1060::PROFILE;
        let mut adapter = PanelAdapter::new(
            profile,
            Ports {
                calls: Vec::new(),
                fail: None,
                cleanup_fail: false,
            },
        );
        let mut selected = selection();
        selected.wiring_confirmed = false;
        assert_eq!(
            adapter.start(&selected),
            Err(Error::Blocked(Blocked::Wiring))
        );
        assert!(adapter.ports.calls.is_empty());
        selected.wiring_confirmed = true;
        adapter.ports.fail = Some("command");
        adapter.ports.cleanup_fail = true;
        assert_eq!(
            adapter.start(&selected),
            Err(Error::Port {
                error: 1,
                cleanup_failed: true
            })
        );
        assert!(
            adapter
                .ports
                .calls
                .ends_with(&["dark", "stop", "reset", "off"])
        );
        assert!(!adapter.ports.calls.contains(&"light"));
        let previous_calls = adapter.ports.calls.len();
        assert_eq!(adapter.start(&selected), Err(Error::CleanupRequired));
        assert_eq!(adapter.ports.calls.len(), previous_calls);
        adapter.ports.fail = None;
        adapter.ports.cleanup_fail = false;
        assert!(!adapter.shutdown());
        adapter.start(&selected).unwrap();
    }
    #[test]
    fn scanout_precedes_light_and_board_payloads_cannot_cross_models() {
        let profile = &crate::jc1060::PROFILE;
        let mut adapter = PanelAdapter::new(
            profile,
            Ports {
                calls: Vec::new(),
                fail: None,
                cleanup_fail: false,
            },
        );
        adapter.start(&selection()).unwrap();
        assert!(adapter.ports.calls.ends_with(&["scan", "light"]));
        assert_eq!(adapter.start(&selection()), Err(Error::AlreadyStarted));
        assert!(!adapter.shutdown());
        let mut wrong = selection();
        wrong.board = BoardId::GuitionJc8012P4a1cIwY;
        assert_eq!(
            adapter.start(&wrong),
            Err(Error::Blocked(Blocked::WrongBoard))
        );
        let touch = TouchSelection {
            board: BoardId::GuitionJc8012P4a1cIwY,
            controller: TouchController::Gsl3680,
            address: 0x40,
            reset_gpio: Some(22),
            interrupt_gpio: Some(21),
            wiring_confirmed: true,
            coordinate_mapping_confirmed: true,
            payload: None,
            payload_sha256: None,
            distribution_authorized: false,
        };
        assert_eq!(
            touch.validate(&crate::jc8012::PROFILE),
            Err(TouchBlocked::Payload)
        );
        assert_eq!(touch.validate(profile), Err(TouchBlocked::WrongBoard));
        assert_eq!(touch_pins(profile.id), TouchPins::Unresolved);
    }
    #[test]
    fn every_panel_port_failure_rolls_back_before_retry_and_rejects_oversized_programs() {
        let profile = &crate::jc1060::PROFILE;
        for step in [
            "dark", "power", "reset", "release", "dsi", "command", "scan", "light",
        ] {
            let mut adapter = PanelAdapter::new(
                profile,
                Ports {
                    calls: Vec::new(),
                    fail: Some(step),
                    cleanup_fail: false,
                },
            );
            assert!(matches!(
                adapter.start(&selection()),
                Err(Error::Port { .. })
            ));
            assert!(
                adapter
                    .ports
                    .calls
                    .ends_with(&["dark", "stop", "reset", "off"])
            );
            if !matches!(step, "light") {
                assert!(!adapter.ports.calls.contains(&"light"));
            }
        }
        let oversized = [0u8; 65];
        let commands = [Command {
            command: 0,
            data: &oversized,
            delay_ms: 0,
        }];
        let mut selected = selection();
        selected.commands = &commands;
        let mut adapter = PanelAdapter::new(
            profile,
            Ports {
                calls: Vec::new(),
                fail: None,
                cleanup_fail: false,
            },
        );
        assert_eq!(
            adapter.start(&selected),
            Err(Error::Blocked(Blocked::MissingTable))
        );
        assert!(adapter.ports.calls.is_empty());
    }
    #[test]
    fn touch_requires_selection_and_cleanup_before_read_or_restart() {
        struct Touch {
            calls: u32,
            fail_init: bool,
            fail_stop: bool,
        }
        impl TouchPort for Touch {
            type Error = u8;
            fn initialize(&mut self, _: &TouchSelection<'_>) -> Result<(), u8> {
                self.calls += 1;
                if self.fail_init { Err(1) } else { Ok(()) }
            }
            fn read_point(&mut self) -> Result<Option<(u16, u16)>, u8> {
                self.calls += 1;
                Ok(Some((4, 5)))
            }
            fn stop(&mut self) -> Result<(), u8> {
                self.calls += 1;
                if self.fail_stop { Err(2) } else { Ok(()) }
            }
        }
        let mut adapter = TouchAdapter::new(
            &crate::jc1060::PROFILE,
            Touch {
                calls: 0,
                fail_init: true,
                fail_stop: true,
            },
        );
        assert_eq!(adapter.read_point(), Err(TouchError::NotStarted));
        let mut selected = TouchSelection {
            board: BoardId::GuitionJc1060P470cIwY,
            controller: TouchController::Gt911,
            address: 0x5d,
            reset_gpio: None,
            interrupt_gpio: None,
            wiring_confirmed: false,
            coordinate_mapping_confirmed: true,
            payload: None,
            payload_sha256: None,
            distribution_authorized: false,
        };
        assert_eq!(
            adapter.start(&selected),
            Err(TouchError::Blocked(TouchBlocked::Wiring))
        );
        assert_eq!(adapter.ports.calls, 0);
        selected.wiring_confirmed = true;
        assert_eq!(adapter.start(&selected), Err(TouchError::Port(1)));
        let calls = adapter.ports.calls;
        assert_eq!(adapter.start(&selected), Err(TouchError::CleanupRequired));
        assert_eq!(adapter.ports.calls, calls);
        adapter.ports.fail_stop = false;
        adapter.stop().unwrap();
        adapter.ports.fail_init = false;
        adapter.start(&selected).unwrap();
        assert_eq!(adapter.read_point(), Ok(Some((4, 5))));
    }
}
