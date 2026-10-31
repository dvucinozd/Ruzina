"""Fresh, device-free build/package; requires an exported pinned IDF environment."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tomllib

PIN = 'fff9895c82d744c7237be8847347bdd1b07c6643'
TARGET = 'riscv32imafc-unknown-none-elf'
APP = Path(__file__).resolve().parents[1]
REPO = APP.parents[1]


def run(args, cwd=None):
    result = subprocess.run([str(x) for x in args], cwd=cwd, text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    if result.returncode:
        raise RuntimeError(result.stdout)
    return result.stdout.strip()


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def sources():
    paths = list(APP.rglob('*'))
    lock = tomllib.loads((APP / 'rust-core/Cargo.lock').read_text())
    for package in lock['package']:
        if package['name'] != 'ruzina-legacy-idf-core-probe':
            paths += list((REPO / 'firmware-rust/crates' / package['name']).rglob('*'))
    paths += [REPO / 'firmware-rust/Cargo.toml', REPO / 'firmware-rust/Cargo.lock']
    return {str(p.relative_to(REPO)).replace('\\', '/'): sha(p) for p in sorted(set(paths))
            if p.is_file() and not any(x in p.relative_to(REPO).parts
                                      for x in ('__pycache__', 'target', 'build'))
            and p.name not in ('sdkconfig', 'sdkconfig.old')}


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--idf', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--variant', choices=['pure-ram', 'flash-build-only', 'memory-task', 'display-touch', 'sd-readonly', 'sd-file-readonly', 'sd-rust-bridge', 'sd-rust-exfat', 'sd-rust-load', 'sd-rust-load-display', 'usb-host', 'usb-midi', 'deck-ui', 'deck-audio'], required=True)
    parser.add_argument('--wsl', help='Optional WSL distribution for Rust only')
    parser.add_argument('--cargo-home')
    parser.add_argument('--rustup-home')
    args = parser.parse_args()
    idf = args.idf.resolve()
    output = args.output.resolve()
    if output.exists():
        raise ValueError('Fresh output directory required')
    if run(['git', 'rev-parse', 'HEAD'], idf) != PIN or run(['git', 'status', '--porcelain'], idf):
        raise ValueError('IDF pin/cleanliness mismatch')
    subs = run(['git', 'submodule', 'status', '--recursive'], idf)
    if any(line and line[0] in '-+U' for line in subs.splitlines()):
        raise ValueError('Submodule pin mismatch')
    run(['git', 'submodule', 'foreach', '--recursive',
         'test -z "$(git status --porcelain)"'], idf)
    import esptool
    if esptool.__version__ != '5.6.0':
        raise ValueError('esptool 5.6.0 required')
    gcc = run(['riscv32-esp-elf-gcc', '--version'])
    if 'esp-15.2.0_20251204' not in gcc:
        raise ValueError('Wrong GCC')
    output.mkdir(parents=True)
    initial_sources = sources()
    def rust(command):
        command = ['cargo', '+1.95.0', *command]
        if args.wsl:
            import shlex
            def unix(path):
                return run(['wsl', '-d', args.wsl, '--', 'wslpath', '-a', path.as_posix()])
            command = [unix(x) if isinstance(x, Path) else x for x in command]
            env = []
            if args.cargo_home:
                env += ['CARGO_HOME=' + args.cargo_home,
                        'PATH=' + args.cargo_home + '/bin:/usr/local/bin:/usr/bin:/bin']
            if args.rustup_home:
                env += ['RUSTUP_HOME=' + args.rustup_home]
            return run(['wsl', '-d', args.wsl, '--', 'env', *env, *command])
        return run(command)
    versions = dict(rust=rust(['--version']), gcc=gcc,
                    cmake=run(['cmake', '--version']), ninja=run(['ninja', '--version']),
                    python=sys.version, esptool=esptool.__version__)
    rust(['fmt', '--manifest-path', APP / 'rust-core/Cargo.toml', '--', '--check'])
    common = ['--locked', '--release', '--target', TARGET, '--manifest-path',
              APP / 'rust-core/Cargo.toml', '--target-dir', output / 'rust']
    if args.variant == 'memory-task':
        common += ['--features', 'memory-task']
    elif args.variant == 'sd-rust-bridge':
        common += ['--features', 'idf-media-bridge']
    elif args.variant == 'sd-rust-exfat':
        common += ['--features', 'idf-exfat-fixture']
    elif args.variant in ('sd-rust-load', 'sd-rust-load-display'):
        common += ['--features', 'idf-media-load']
    elif args.variant == 'usb-midi':
        common += ['--features', 'idf-controller-midi']
    elif args.variant == 'deck-ui':
        common += ['--features', 'idf-media-load idf-controller-midi']
    elif args.variant == 'deck-audio':
        common += ['--features', 'idf-media-load idf-controller-audio']
    rust(['clippy', *common, '--', '-D', 'warnings'])
    (output / 'rust-build.log').write_text(rust(['build', *common]))
    archive = output / 'rust' / TARGET / 'release/libruzina_legacy_idf_core_probe.a'
    defaults = [APP / 'sdkconfig.defaults']
    if args.variant == 'pure-ram':
        defaults += [APP / 'sdkconfig.ram.defaults']
    elif args.variant == 'memory-task':
        defaults += [APP / 'sdkconfig.memory.defaults']
    elif args.variant == 'display-touch':
        defaults += [APP / 'sdkconfig.display.defaults']
    elif args.variant == 'sd-rust-bridge':
        defaults += [APP / 'sdkconfig.sd-bridge.defaults']
    elif args.variant == 'sd-rust-exfat':
        defaults += [APP / 'sdkconfig.sd-rust-exfat.defaults']
    elif args.variant in ('usb-host', 'usb-midi'):
        defaults += [APP / 'sdkconfig.usb-host.defaults']
        if args.variant == 'usb-midi':
            defaults += [APP / 'sdkconfig.usb-midi.defaults']
    elif args.variant in ('sd-rust-load', 'sd-rust-load-display', 'deck-ui', 'deck-audio'):
        defaults += [APP / 'sdkconfig.sd-rust-load.defaults']
        if args.variant in ('sd-rust-load-display', 'deck-ui', 'deck-audio'):
            defaults += [APP / 'sdkconfig.load-display.defaults']
        if args.variant in ('deck-ui', 'deck-audio'):
            defaults += [APP / 'sdkconfig.deck-ui.defaults']
        if args.variant == 'deck-audio':
            defaults += [APP / 'sdkconfig.deck-audio.defaults']
    elif args.variant in ('sd-readonly', 'sd-file-readonly'):
        defaults += [APP / 'sdkconfig.sd.defaults']
        if args.variant == 'sd-file-readonly':
            defaults += [APP / 'sdkconfig.sd-file.defaults']
    os.environ['IDF_PATH'] = str(idf)
    os.environ['ESP_IDF_VERSION'] = '6.1'
    # Offline build: vendored components only; the component manager would
    # otherwise resolve their manifests and write dependencies.lock here.
    os.environ['IDF_COMPONENT_MANAGER'] = '0'
    command = [sys.executable, idf / 'tools/idf.py', '-C', APP, '-B', output / 'build',
               '-DSDKCONFIG=' + (output / 'sdkconfig').as_posix(),
               '-DSDKCONFIG_DEFAULTS=' + ';'.join(x.as_posix() for x in defaults),
               '-DRUZINA_RUST_ARCHIVE=' + archive.as_posix(), 'build']
    (output / 'idf-build.log').write_text(run(command))
    config = (output / 'sdkconfig').read_text()
    for required in ('CONFIG_ESP32P4_REV_MIN_FULL=100', 'CONFIG_ESP32P4_REV_MAX_FULL=199',
                     'CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ=360'):
        if required not in config:
            raise ValueError('Invalid generated config: ' + required)
    if args.variant == 'memory-task':
        for required in ('CONFIG_SPIRAM_SPEED=80', 'CONFIG_SPIRAM_USE_CAPS_ALLOC=y', 'CONFIG_RUZINA_MEMORY_TASK=y'):
            if required not in config:
                raise ValueError('Wrong memory profile: ' + required)
    if args.variant in ('sd-rust-load', 'sd-rust-load-display', 'deck-ui', 'deck-audio'):
        # The Rust allocator takes every allocation from caps-allocated PSRAM.
        for required in ('CONFIG_SPIRAM=y', 'CONFIG_SPIRAM_SPEED=80', 'CONFIG_SPIRAM_USE_CAPS_ALLOC=y',
                         'CONFIG_SPIRAM_BOOT_INIT=y', 'CONFIG_RUZINA_SD_RUST_LOAD=y'):
            if required not in config:
                raise ValueError('Wrong load profile: ' + required)
    # Only the USB host diagnostic may enable and link the USB Host Library.
    usb_variant = args.variant in ('usb-host', 'usb-midi', 'deck-ui', 'deck-audio')
    if ('CONFIG_RUZINA_USB_HOST_ENUM=y' in config) != usb_variant:
        raise ValueError('USB host profile mismatch')
    if ('CONFIG_RUZINA_USB_HOST_MIDI=y' in config) != (args.variant in ('usb-midi', 'deck-ui', 'deck-audio')):
        raise ValueError('USB-MIDI profile mismatch')
    if ('CONFIG_RUZINA_USB_HOST_UAC=y' in config) != (args.variant == 'deck-audio'):
        raise ValueError('UAC profile mismatch')
    if usb_variant and 'CONFIG_USB_HOST_CONTROL_TRANSFER_MAX_SIZE=4096' not in config:
        raise ValueError('USB host control buffer too small for audio-class configurations')
    from esptool.bin_image import LoadFirmwareImage
    binary = output / 'build/ruzina_legacy_probe.bin'
    data = binary.read_bytes()
    image = LoadFirmwareImage('ESP32-P4', data)
    if image.chip_id != 18 or (image.min_rev_full, image.max_rev_full) != (100, 199):
        raise ValueError('Wrong image header')
    if args.variant == 'pure-ram':
        from run_ram_probe import validate_segments
        validate_segments(image.entrypoint, [(s.addr, len(s.data)) for s in image.segments])
    mapfile = output / 'build/ruzina_legacy_probe.map'
    mapping = mapfile.read_text()
    if 'ruzina_legacy_pitch_centipercent' not in mapping or 'ilp32f/libgcc.a' not in mapping.replace('\\', '/'):
        raise ValueError('Missing Rust symbol or matching runtime in map')
    elf = output / 'build/ruzina_legacy_probe.elf'
    abi = run(['riscv32-esp-elf-readelf', '-h', elf])
    if 'single-float ABI' not in abi:
        raise ValueError('Wrong linked ELF ABI')
    links = '\n'.join(p.read_text(errors='replace') for p in (output / 'build').rglob('*.ninja'))
    # IDF itself has C components named esp_hal_*; only Rust startup archives
    # (with Cargo's hash suffix) are forbidden here.
    if any(x in links for x in ('--no-warn-mismatch', 'libesp_hal-', 'libesp_rtos-')):
        raise ValueError('Forbidden linker flag/startup')
    if args.variant in ('usb-host', 'usb-midi'):
        if args.variant == 'usb-midi' and 'ruzina_legacy_midi_ingest' not in run(['riscv32-esp-elf-nm', '--defined-only', elf]):
            raise ValueError('Rust USB-MIDI session not linked')
        if args.variant == 'usb-midi':
            # The Rust MIDI calls run on the 8 KiB USB client task; a lazily
            # built session once put a 5.4 KiB frame there and overflowed it.
            from stack_frames import oversized_rust_frames
            oversized = oversized_rust_frames(run(['riscv32-esp-elf-objdump', '-d', '--no-show-raw-insn', elf]),
                                              limit=2048)
            if oversized:
                raise ValueError('Rust frame over the USB client task budget: ' + repr(oversized))
        if 'usb_host_enum.c.obj' not in mapping or 'libusb.a' not in mapping.replace('\\', '/'):
            raise ValueError('USB host diagnostic or library not linked')
        if any(x in mapping for x in ('sd_bridge.c.obj', 'load_display.c.obj', 'display_touch.c.obj')):
            raise ValueError('USB host diagnostic must stay isolated')
    elif args.variant in ('deck-ui', 'deck-audio'):
        # The one combined profile: SD load, status display and USB-MIDI.
        symbols = run(['riscv32-esp-elf-nm', '--defined-only', elf])
        if any(x not in symbols for x in ('ruzina_legacy_midi_ingest', 'ruzina_legacy_load_run', 'usb_host_install')):
            raise ValueError('Deck UI must link the Rust load run, the MIDI session and the USB host')
        if args.variant == 'deck-audio' and 'ruzina_legacy_uac_fill' not in symbols:
            raise ValueError('Deck audio must link the Rust UAC packer')
        if any(x not in mapping for x in ('usb_host_enum.c.obj', 'load_display.c.obj', 'sd_bridge.c.obj')):
            raise ValueError('Deck UI owner sources missing')
        # MIDI exports run on the 8 KiB USB client task (the SD client has
        # its own 48 KiB stack); keep their frames within 2 KiB.
        from stack_frames import frame_sizes
        frames = frame_sizes(run(['riscv32-esp-elf-objdump', '-d', '--no-show-raw-insn', elf]))
        midi = {k: v for k, v in frames.items()
                if k.startswith(('ruzina_legacy_midi', 'ruzina_legacy_uac')) and v > 2048}
        if midi:
            raise ValueError('MIDI frame over the USB client task budget: ' + repr(midi))
    elif 'usb_host_install' in run(['riscv32-esp-elf-nm', '--defined-only', elf]):
        raise ValueError('USB Host Library linked outside the USB host profile')
    if sources() != initial_sources:
        raise ValueError('Sources changed during build')
    artifacts = [binary, elf, mapfile, output / 'sdkconfig', archive,
                 output / 'build/bootloader/bootloader.bin',
                 output / 'build/partition_table/partition-table.bin',
                 output / 'build/flasher_args.json']
    if args.variant in ('sd-readonly', 'sd-file-readonly', 'sd-rust-bridge', 'sd-rust-exfat', 'sd-rust-load', 'sd-rust-load-display', 'deck-ui', 'deck-audio'):
        symbols = run(['riscv32-esp-elf-nm', '--defined-only', elf])
        forbidden = {'f_write', 'f_mkfs', 'f_unlink', 'f_rename', 'sdmmc_write_sectors', 'sdmmc_erase_sectors'}
        if forbidden.intersection(line.split()[-1] for line in symbols.splitlines() if line.split()) or 'libfatfs.a' in mapping:
            raise ValueError('Read/write filesystem or SD write/erase API linked')
        (output / 'readonly-symbols.log').write_text(symbols)
        if args.variant in ('sd-rust-bridge', 'sd-rust-exfat', 'sd-rust-load', 'sd-rust-load-display', 'deck-ui', 'deck-audio'):
            if 'CONFIG_RUZINA_SD_RUST_BRIDGE=y' not in config or any(x in config for x in ('CONFIG_RUZINA_SD_READONLY=y', 'CONFIG_RUZINA_SD_FIXTURE=y')):
                raise ValueError('Wrong exclusive Rust bridge configuration')
            required_symbol={'sd-rust-exfat': 'ruzina_legacy_exfat_run', 'sd-rust-load': 'ruzina_legacy_load_run', 'sd-rust-load-display': 'ruzina_legacy_load_run', 'deck-ui': 'ruzina_legacy_load_run', 'deck-audio': 'ruzina_legacy_load_run'}.get(args.variant, 'ruzina_legacy_bridge_run')
            if required_symbol not in symbols or 'sd_bridge.c.obj' not in mapping or any(x in symbols.split() for x in ('f_open', 'f_mount')):
                raise ValueError('Missing Rust bridge or unexpected filesystem linked')
            if ('CONFIG_RUZINA_SD_RUST_EXFAT=y' in config) != (args.variant == 'sd-rust-exfat'):
                raise ValueError('Rust exFAT profile/feature mismatch')
            if ('CONFIG_RUZINA_SD_RUST_LOAD=y' in config) != (args.variant in ('sd-rust-load', 'sd-rust-load-display', 'deck-ui', 'deck-audio')):
                raise ValueError('Rust load profile/feature mismatch')
            if ('CONFIG_RUZINA_LOAD_DISPLAY=y' in config) != (args.variant in ('sd-rust-load-display', 'deck-ui', 'deck-audio')) or (
                    args.variant in ('sd-rust-load-display', 'deck-ui', 'deck-audio') and ('load_display.c.obj' not in mapping
                                                                or 'CONFIG_RUZINA_DISPLAY_TOUCH=y' in config)):
                raise ValueError('Load display profile mismatch')
            from stack_frames import oversized_rust_frames
            oversized = oversized_rust_frames(run(['riscv32-esp-elf-objdump', '-d', '--no-show-raw-insn', elf]))
            if oversized:
                raise ValueError('Rust stack frame over task budget: ' + repr(oversized))
    manifest = dict(schema=1, variant=args.variant, sha256=sha(binary), bytes=len(data),
                    entrypoint=image.entrypoint, segments=[dict(address=s.addr, bytes=len(s.data)) for s in image.segments],
                    idf_commit=PIN, submodules=subs, versions=versions, sources=initial_sources,
                    fatfs_sources={name: sha(idf / 'components/fatfs/src' / name) for name in ('ff.c','ff.h','ffunicode.c','diskio.h')} if args.variant in ('sd-readonly', 'sd-file-readonly') else {},
                    source_commit=run(['git', 'rev-parse', 'HEAD'], REPO),
                    dirty_status=run(['git', 'status', '--porcelain'], REPO),
                    diff_sha256=hashlib.sha256(run(['git', 'diff', '--binary', 'HEAD'], REPO).encode()).hexdigest(),
                    artifacts={str(p.relative_to(output)).replace('\\', '/'): sha(p)
                               for p in artifacts if p.is_file()},
                    readiness='BUILD_ONLY_NOT_QUALIFIED')
    (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print('BUILD_PACKAGE_PASS ' + str(output / 'manifest.json'))


if __name__ == '__main__':
    main()
