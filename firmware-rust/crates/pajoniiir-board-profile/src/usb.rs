//! Board-owned USB roles (RP-10). A device's role comes from the physical
//! port the qualified board plan assigns, never from enumeration order or a
//! donor root-port number. Storage belongs on the qualified high-speed path.
//! Both Guition plans are unqualified until their connectors, host power and
//! FIFO budgets are measured, so every role is refused today.

use crate::BoardId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbPort {
    HighSpeed,
    FullSpeed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceKind {
    MassStorage,
    /// MIDI and optional UAC.
    Controller,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortBudget {
    pub port: UsbPort,
    /// Host FIFO bytes reserved for the role's periodic/bulk traffic.
    pub fifo_bytes: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsbRolePlan {
    pub storage: Option<PortBudget>,
    pub controller: Option<PortBudget>,
    /// Set only by a BSP whose ports, VBUS policy and budgets were measured.
    pub qualified: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoleRefusal {
    BoardUnqualified,
    /// The plan gives this kind of device no port.
    NoRole,
    /// The device sits on another port than its role's.
    WrongPort,
}

impl UsbRolePlan {
    /// A plan is usable only with storage on high speed and distinct ports.
    pub fn validate(&self) -> bool {
        self.storage
            .is_none_or(|storage| storage.port == UsbPort::HighSpeed)
            && !matches!((self.storage, self.controller), (Some(a), Some(b)) if a.port == b.port)
    }

    pub fn assign(&self, kind: DeviceKind, port: UsbPort) -> Result<PortBudget, RoleRefusal> {
        if !self.qualified || !self.validate() {
            return Err(RoleRefusal::BoardUnqualified);
        }
        let budget = match kind {
            DeviceKind::MassStorage => self.storage,
            DeviceKind::Controller => self.controller,
        }
        .ok_or(RoleRefusal::NoRole)?;
        if budget.port != port {
            return Err(RoleRefusal::WrongPort);
        }
        Ok(budget)
    }
}

/// The audited bundles document a HS and a FS connector sharing USB5V_IN, but
/// no FS data mapping for v3.2 and no host-power policy; nothing is assigned.
pub const fn usb_role_plan(id: BoardId) -> UsbRolePlan {
    match id {
        BoardId::GuitionJc1060P470cIwY | BoardId::GuitionJc8012P4a1cIwY => UsbRolePlan {
            storage: None,
            controller: None,
            qualified: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUALIFIED: UsbRolePlan = UsbRolePlan {
        storage: Some(PortBudget {
            port: UsbPort::HighSpeed,
            fifo_bytes: 2048,
        }),
        controller: Some(PortBudget {
            port: UsbPort::FullSpeed,
            fifo_bytes: 640,
        }),
        qualified: true,
    };

    #[test]
    fn both_guition_boards_refuse_every_role_until_qualified() {
        for id in [
            BoardId::GuitionJc1060P470cIwY,
            BoardId::GuitionJc8012P4a1cIwY,
        ] {
            let plan = usb_role_plan(id);
            for kind in [DeviceKind::MassStorage, DeviceKind::Controller] {
                for port in [UsbPort::HighSpeed, UsbPort::FullSpeed] {
                    assert_eq!(plan.assign(kind, port), Err(RoleRefusal::BoardUnqualified));
                }
            }
        }
    }

    #[test]
    fn roles_follow_the_physical_port_not_arrival() {
        assert_eq!(
            QUALIFIED
                .assign(DeviceKind::MassStorage, UsbPort::HighSpeed)
                .map(|b| b.fifo_bytes),
            Ok(2048)
        );
        assert_eq!(
            QUALIFIED.assign(DeviceKind::MassStorage, UsbPort::FullSpeed),
            Err(RoleRefusal::WrongPort),
            "storage on the FS port is refused, even if it enumerated first"
        );
        assert_eq!(
            QUALIFIED.assign(DeviceKind::Controller, UsbPort::HighSpeed),
            Err(RoleRefusal::WrongPort)
        );
        let storage_only = UsbRolePlan {
            controller: None,
            ..QUALIFIED
        };
        assert_eq!(
            storage_only.assign(DeviceKind::Controller, UsbPort::FullSpeed),
            Err(RoleRefusal::NoRole)
        );
        for invalid in [
            UsbRolePlan {
                storage: Some(PortBudget {
                    port: UsbPort::FullSpeed,
                    fifo_bytes: 512,
                }),
                controller: None,
                qualified: true,
            },
            UsbRolePlan {
                controller: QUALIFIED.storage,
                ..QUALIFIED
            },
        ] {
            assert!(!invalid.validate());
            assert_eq!(
                invalid.assign(DeviceKind::MassStorage, UsbPort::HighSpeed),
                Err(RoleRefusal::BoardUnqualified)
            );
        }
    }
}
