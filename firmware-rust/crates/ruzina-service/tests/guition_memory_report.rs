//! Serialize the actual composed boot record through the device's exact buffer
//! bound. This prevents a new optional probe from silently suppressing all USB
//! diagnostics when the combined output no longer fits.
use core::fmt::Write;
use ruzina_board_profile::{
    BoardId,
    diagnostics::{BootReport, ReportBuffer},
    memory::{FlashProbe, MemoryReport, Mismatch, PsramProbe},
    profile,
};
use ruzina_service::{Environment, Identity, RuntimeStatus, Service};

#[test]
fn combined_device_report_fits_and_preserves_probe_failures_for_each_board() {
    for id in [
        BoardId::GuitionJc1060P470cIwY,
        BoardId::GuitionJc8012P4a1cIwY,
    ] {
        for memory in [
            MemoryReport::default(),
            MemoryReport {
                flash: FlashProbe::ReadId(0x1840ef),
                psram: PsramProbe::ScratchPassed {
                    mapped_bytes: 33554432,
                },
            },
            MemoryReport {
                flash: FlashProbe::Timeout,
                psram: PsramProbe::ScratchFailed {
                    mapped_bytes: 33554432,
                    mismatch: Mismatch {
                        word: 16383,
                        expected: u32::MAX,
                        actual: 0,
                    },
                },
            },
        ] {
            let profile = profile(id);
            let image = "8bc11e3d06a6-src-0123456789abcdef-jc1060-memory";
            let mut output = ReportBuffer::<2048>::default();
            BootReport {
                profile,
                chip_revision: 302,
                image_id: image,
                observed_flash_bytes: memory.flash.bytes(),
                observed_psram_bytes: memory.psram.bytes(),
            }
            .write_to(&mut output)
            .unwrap();
            writeln!(output, "reset_reason=Some(SoftwareCpu)").unwrap();
            let mut service = Service::new(Identity {
                environment: Environment::DeviceBootSnapshot,
                board: id,
                image_id: Some(image),
                chip_revision: Some(302),
                flash_bytes: memory.flash.bytes(),
                psram_bytes: memory.psram.bytes(),
                reset_code: Some(3),
            });
            service.advance(0);
            service
                .snapshot(RuntimeStatus::BOOT_ONLY)
                .write_to(&mut output)
                .unwrap();
            memory.write_to(&mut output, profile.model, image).unwrap();
            let text = core::str::from_utf8(output.bytes()).unwrap();
            assert!(text.contains("M1_BOOT_DIAGNOSTIC_V1\n"));
            assert!(text.contains("M1_MEMORY_DIAGNOSTIC_V1\n"));
            assert!(text.ends_with("readiness=NOT_QUALIFIED\n"));
            if memory.flash == FlashProbe::Timeout {
                assert!(text.contains("flash_probe=TIMEOUT\n"));
                assert!(text.contains("psram_failure_word=16383\n"));
            }
        }
    }
}
