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
    parser.add_argument('--variant', choices=['pure-ram', 'flash-build-only', 'memory-task', 'display-touch', 'sd-readonly', 'sd-file-readonly', 'sd-rust-bridge', 'sd-rust-exfat', 'sd-rust-load', 'sd-rust-load-display', 'usb-host', 'usb-midi', 'deck-ui', 'deck-audio', 'deck-play', 'deck-mix', 'deck-engine'], required=True)
    parser.add_argument('--wsl', help='Optional WSL distribution for Rust only')
    parser.add_argument('--cargo-home')
    parser.add_argument('--rustup-home')
    parser.add_argument('--uac-profile', action='store_true', help='Explicit deck-engine-only callback profiling diagnostic')
    args = parser.parse_args()
    if args.uac_profile and args.variant != 'deck-engine':
        raise ValueError('UAC profiling is an explicit deck-engine diagnostic only')
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
    elif args.variant == 'deck-play':
        common += ['--features', 'idf-deck-play']
    elif args.variant == 'deck-mix':
        common += ['--features', 'idf-deck-mix']
    elif args.variant == 'deck-engine':
        common += ['--features', 'idf-uac-profile' if args.uac_profile else 'idf-deck-engine']
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
    elif args.variant in ('sd-rust-load', 'sd-rust-load-display', 'deck-ui', 'deck-audio', 'deck-play', 'deck-mix', 'deck-engine'):
        defaults += [APP / 'sdkconfig.sd-rust-load.defaults']
        if args.variant in ('sd-rust-load-display', 'deck-ui', 'deck-audio', 'deck-play', 'deck-mix', 'deck-engine'):
            defaults += [APP / 'sdkconfig.load-display.defaults']
        if args.variant in ('deck-ui', 'deck-audio', 'deck-play', 'deck-mix', 'deck-engine'):
            defaults += [APP / 'sdkconfig.deck-ui.defaults']
        if args.variant in ('deck-audio', 'deck-play', 'deck-mix', 'deck-engine'):
            defaults += [APP / 'sdkconfig.deck-audio.defaults']
        if args.variant in ('deck-play', 'deck-mix', 'deck-engine'):
            defaults += [APP / 'sdkconfig.deck-play.defaults']
        if args.variant in ('deck-mix', 'deck-engine'):
            defaults += [APP / 'sdkconfig.deck-mix.defaults']
        if args.variant == 'deck-engine':
            defaults += [APP / 'sdkconfig.deck-engine.defaults']
    elif args.variant in ('sd-readonly', 'sd-file-readonly'):
        defaults += [APP / 'sdkconfig.sd.defaults']
        if args.variant == 'sd-file-readonly':
            defaults += [APP / 'sdkconfig.sd-file.defaults']
    if args.uac_profile:
        defaults += [APP / 'sdkconfig.uac-profile.defaults']
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
    if ('CONFIG_RUZINA_UAC_PROFILE=y' in config) != args.uac_profile:
        raise ValueError('UAC profiling configuration mismatch')
    for required in ('CONFIG_ESP32P4_REV_MIN_FULL=100', 'CONFIG_ESP32P4_REV_MAX_FULL=199',
                     'CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ=360'):
        if required not in config:
            raise ValueError('Invalid generated config: ' + required)
    if args.variant == 'memory-task':
        for required in ('CONFIG_SPIRAM_SPEED=80', 'CONFIG_SPIRAM_USE_CAPS_ALLOC=y', 'CONFIG_RUZINA_MEMORY_TASK=y'):
            if required not in config:
                raise ValueError('Wrong memory profile: ' + required)
    if args.variant in ('sd-rust-load', 'sd-rust-load-display', 'deck-ui', 'deck-audio', 'deck-play', 'deck-mix', 'deck-engine'):
        # The Rust allocator takes every allocation from caps-allocated PSRAM.
        for required in ('CONFIG_SPIRAM=y', 'CONFIG_SPIRAM_SPEED=80', 'CONFIG_SPIRAM_USE_CAPS_ALLOC=y',
                         'CONFIG_SPIRAM_BOOT_INIT=y', 'CONFIG_RUZINA_SD_RUST_LOAD=y'):
            if required not in config:
                raise ValueError('Wrong load profile: ' + required)
    # Only the USB host diagnostic may enable and link the USB Host Library.
    usb_variant = args.variant in ('usb-host', 'usb-midi', 'deck-ui', 'deck-audio', 'deck-play', 'deck-mix', 'deck-engine')
    if ('CONFIG_RUZINA_USB_HOST_ENUM=y' in config) != usb_variant:
        raise ValueError('USB host profile mismatch')
    if ('CONFIG_RUZINA_USB_HOST_MIDI=y' in config) != (args.variant in ('usb-midi', 'deck-ui', 'deck-audio', 'deck-play', 'deck-mix', 'deck-engine')):
        raise ValueError('USB-MIDI profile mismatch')
    if ('CONFIG_RUZINA_USB_HOST_UAC=y' in config) != (args.variant in ('deck-audio', 'deck-play', 'deck-mix', 'deck-engine')):
        raise ValueError('UAC profile mismatch')
    if ('CONFIG_RUZINA_DECK_PLAY=y' in config) != (args.variant in ('deck-play', 'deck-mix', 'deck-engine')):
        raise ValueError('Deck playback profile mismatch')
    if ('CONFIG_RUZINA_DECK_MIX=y' in config) != (args.variant in ('deck-mix', 'deck-engine')):
        raise ValueError('Deck mix profile mismatch')
    if ('CONFIG_RUZINA_DECK_ENGINE=y' in config) != (args.variant == 'deck-engine'):
        raise ValueError('Deck engine profile mismatch')
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
    profile_symbols = run(['riscv32-esp-elf-nm', '--defined-only', elf])
    for symbol in ('ruzina_legacy_uac_profile_begin', 'ruzina_legacy_uac_profile_end',
                   'ruzina_uac_switched_in', 'ruzina_uac_switched_out'):
        if (symbol in profile_symbols) != args.uac_profile:
            raise ValueError('Profiling symbol/default isolation mismatch: ' + symbol)
    if args.uac_profile:
        tasks_disassembly = run(['riscv32-esp-elf-objdump', '-dr', output / 'build/esp-idf/freertos/CMakeFiles/__idf_freertos.dir/FreeRTOS-Kernel/tasks.c.obj'])
        if any(s not in tasks_disassembly for s in ('ruzina_uac_switched_in', 'ruzina_uac_switched_out')):
            raise ValueError('FreeRTOS task switch hooks are not actually compiled into the kernel')
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
    elif args.variant in ('deck-ui', 'deck-audio', 'deck-play', 'deck-mix', 'deck-engine'):
        # The one combined profile: SD load, status display and USB-MIDI.
        symbols = run(['riscv32-esp-elf-nm', '--defined-only', elf])
        if any(x not in symbols for x in ('ruzina_legacy_midi_ingest', 'ruzina_legacy_load_run', 'usb_host_install')):
            raise ValueError('Deck UI must link the Rust load run, the MIDI session and the USB host')
        if args.variant in ('deck-audio', 'deck-play', 'deck-mix', 'deck-engine') and 'ruzina_legacy_uac_fill' not in symbols:
            raise ValueError('Deck audio must link the Rust UAC packer')
        if args.variant == 'deck-play' and any(x not in symbols for x in ('ruzina_legacy_play_load', 'ruzina_legacy_play_stats')):
            raise ValueError('Deck playback must link the Rust deck 1 player')
        if args.variant in ('deck-mix', 'deck-engine') and any(x not in symbols for x in ('ruzina_legacy_play_load', 'ruzina_legacy_mix_stats')):
            raise ValueError('Deck mix must link the Rust deck players and render graph owner')
        if args.variant == 'deck-engine' and any(x not in symbols for x in ('ruzina_legacy_engine_status', 'ruzina_legacy_midi_refresh')):
            raise ValueError('Deck engine must link the Rust deck engine status and LED refresh')
        if any(x not in mapping for x in ('usb_host_enum.c.obj', 'load_display.c.obj', 'sd_bridge.c.obj')):
            raise ValueError('Deck UI owner sources missing')
        # MIDI, UAC, playback and mix exports run on the 8 KiB USB client
        # task (the SD client has its own stack); keep their frames within
        # 2 KiB.
        from stack_frames import deepest_stack, frame_sizes
        disassembly = run(['riscv32-esp-elf-objdump', '-d', '--no-show-raw-insn', elf])
        frames = frame_sizes(disassembly)
        midi = {k: v for k, v in frames.items()
                if k.startswith(('ruzina_legacy_midi', 'ruzina_legacy_uac', 'ruzina_legacy_play', 'ruzina_legacy_mix',
                                 'ruzina_legacy_engine'))
                and v > 2048}
        if midi:
            raise ValueError('MIDI frame over the USB client task budget: ' + repr(midi))
        if args.variant in ('deck-mix', 'deck-engine'):
            # The render graph runs inside the UAC fill on that 8 KiB task,
            # under the C client loop and the USB host callbacks.
            depth, path = deepest_stack(disassembly, 'ruzina_legacy_uac_fill')
            (output / 'uac-fill-stack.log').write_text(f'{depth}\n' + '\n'.join(path) + '\n')
            if depth > 4096:
                raise ValueError(f'UAC fill call chain needs {depth} bytes of the 8 KiB USB client stack')
    elif 'usb_host_install' in run(['riscv32-esp-elf-nm', '--defined-only', elf]):
        raise ValueError('USB Host Library linked outside the USB host profile')
    if sources() != initial_sources:
        raise ValueError('Sources changed during build')
    artifacts = [binary, elf, mapfile, output / 'sdkconfig', archive,
                 output / 'build/bootloader/bootloader.bin',
                 output / 'build/partition_table/partition-table.bin',
                 output / 'build/flasher_args.json']
    if args.variant in ('sd-readonly', 'sd-file-readonly', 'sd-rust-bridge', 'sd-rust-exfat', 'sd-rust-load', 'sd-rust-load-display', 'deck-ui', 'deck-audio', 'deck-play', 'deck-mix', 'deck-engine'):
        symbols = run(['riscv32-esp-elf-nm', '--defined-only', elf])
        forbidden = {'f_write', 'f_mkfs', 'f_unlink', 'f_rename', 'sdmmc_write_sectors', 'sdmmc_erase_sectors'}
        if forbidden.intersection(line.split()[-1] for line in symbols.splitlines() if line.split()) or 'libfatfs.a' in mapping:
            raise ValueError('Read/write filesystem or SD write/erase API linked')
        (output / 'readonly-symbols.log').write_text(symbols)
        if args.variant in ('sd-rust-bridge', 'sd-rust-exfat', 'sd-rust-load', 'sd-rust-load-display', 'deck-ui', 'deck-audio', 'deck-play', 'deck-mix', 'deck-engine'):
            if 'CONFIG_RUZINA_SD_RUST_BRIDGE=y' not in config or any(x in config for x in ('CONFIG_RUZINA_SD_READONLY=y', 'CONFIG_RUZINA_SD_FIXTURE=y')):
                raise ValueError('Wrong exclusive Rust bridge configuration')
            required_symbol={'sd-rust-exfat': 'ruzina_legacy_exfat_run', 'sd-rust-load': 'ruzina_legacy_load_run', 'sd-rust-load-display': 'ruzina_legacy_load_run', 'deck-ui': 'ruzina_legacy_load_run', 'deck-audio': 'ruzina_legacy_load_run', 'deck-play': 'ruzina_legacy_load_run', 'deck-mix': 'ruzina_legacy_load_run', 'deck-engine': 'ruzina_legacy_load_run'}.get(args.variant, 'ruzina_legacy_bridge_run')
            if required_symbol not in symbols or 'sd_bridge.c.obj' not in mapping or any(x in symbols.split() for x in ('f_open', 'f_mount')):
                raise ValueError('Missing Rust bridge or unexpected filesystem linked')
            if ('CONFIG_RUZINA_SD_RUST_EXFAT=y' in config) != (args.variant == 'sd-rust-exfat'):
                raise ValueError('Rust exFAT profile/feature mismatch')
            if ('CONFIG_RUZINA_SD_RUST_LOAD=y' in config) != (args.variant in ('sd-rust-load', 'sd-rust-load-display', 'deck-ui', 'deck-audio', 'deck-play', 'deck-mix', 'deck-engine')):
                raise ValueError('Rust load profile/feature mismatch')
            if ('CONFIG_RUZINA_LOAD_DISPLAY=y' in config) != (args.variant in ('sd-rust-load-display', 'deck-ui', 'deck-audio', 'deck-play', 'deck-mix', 'deck-engine')) or (
                    args.variant in ('sd-rust-load-display', 'deck-ui', 'deck-audio', 'deck-play', 'deck-mix', 'deck-engine') and ('load_display.c.obj' not in mapping
                                                                or 'CONFIG_RUZINA_DISPLAY_TOUCH=y' in config)):
                raise ValueError('Load display profile mismatch')
            from stack_frames import oversized_rust_frames
            oversized = oversized_rust_frames(run(['riscv32-esp-elf-objdump', '-d', '--no-show-raw-insn', elf]))
            if oversized:
                raise ValueError('Rust stack frame over task budget: ' + repr(oversized))
            if args.variant in ('deck-play', 'deck-mix', 'deck-engine'):
                # The decoders build 24 KiB values on the stack (nanomp3's
                # Mp3Dec, MP3 seeks), several frames deep under the load
                # run: the SD client gets 96 KiB here; keep 8 KiB for C.
                from stack_frames import deepest_stack
                depth, path = deepest_stack(run(['riscv32-esp-elf-objdump', '-d', '--no-show-raw-insn', elf]),
                                            'ruzina_legacy_load_run')
                (output / 'load-run-stack.log').write_text(f'{depth}\n' + '\n'.join(path) + '\n')
                if depth > 90112:
                    raise ValueError(f'Load run call chain needs {depth} bytes of the 96 KiB SD client stack')
    manifest = dict(schema=1, variant=args.variant, uac_profile=args.uac_profile, sha256=sha(binary), bytes=len(data),
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
