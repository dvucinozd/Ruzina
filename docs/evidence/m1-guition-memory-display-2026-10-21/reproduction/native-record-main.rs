use core::fmt::Write;
use pajoniiir_board_profile::{BoardId, diagnostics::{BootReport, ReportBuffer}, memory::{FlashProbe, MemoryReport, Mismatch, PsramProbe}, profile};
use pajoniiir_service::{Environment, Identity, RuntimeStatus, Service};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert_eq!(args.len(), 5);
    let id = match args[1].as_str() { "jc1060" => BoardId::GuitionJc1060P470cIwY, "jc8012" => BoardId::GuitionJc8012P4a1cIwY, _ => panic!("board") };
    let image: &'static str = Box::leak(args[2].clone().into_boxed_str());
    let flash = matches!(args[3].as_str(), "flash" | "memory");
    let psram = matches!(args[3].as_str(), "psram" | "memory");
    let failed = args[4] == "failure";
    let memory = MemoryReport {
        flash: if flash { if failed { FlashProbe::Timeout } else { FlashProbe::ReadId(0x1840ef) } } else { FlashProbe::NotRequested },
        psram: if psram { if failed { PsramProbe::ScratchFailed { mapped_bytes: 33554432, mismatch: Mismatch { word: 42, expected: 0xaaaaaaba, actual: 0 } } } else { PsramProbe::ScratchPassed { mapped_bytes: 33554432 } } } else { PsramProbe::NotRequested },
    };
    let profile = profile(id);
    let revision = if failed && !flash && !psram { 301 } else { 302 };
    let mut output = ReportBuffer::<2048>::default();
    BootReport { profile, chip_revision: revision, image_id: image, observed_flash_bytes: memory.flash.bytes(), observed_psram_bytes: memory.psram.bytes() }.write_to(&mut output).unwrap();
    writeln!(output,"reset_reason=Some(SoftwareCpu)").unwrap();
    let mut service = Service::new(Identity { environment: Environment::DeviceBootSnapshot, board: id,
        image_id: Some(image), chip_revision: Some(revision), flash_bytes: memory.flash.bytes(), psram_bytes: memory.psram.bytes(), reset_code: Some(3) });
    service.advance(0);
    service.snapshot(RuntimeStatus::BOOT_ONLY).write_to(&mut output).unwrap();
    memory.write_to(&mut output, profile.model, image).unwrap();
    use std::io::Write as _;
    std::io::stdout().write_all(output.bytes()).unwrap();
}
