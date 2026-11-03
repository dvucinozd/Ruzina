"""Refresh the reviewed primary-source hash index; no build/device access."""
from pathlib import Path
import hashlib,json,subprocess

repo=Path(__file__).resolve().parents[4]
idf=Path('D:/BoardEvidence/JC1060-20261030-COM3/toolchain/esp-idf-v6.1')
pin='fff9895c82d744c7237be8847347bdd1b07c6643'
assert subprocess.check_output(['git','rev-parse','HEAD'],cwd=idf,text=True).strip()==pin
assert not subprocess.check_output(['git','status','--porcelain'],cwd=idf,text=True).strip()
files=[
 'firmware-idf/legacy-jc1060/main/uac_driver.inc',
 'firmware-idf/legacy-jc1060/main/usb_host_enum.c',
 'firmware-idf/legacy-jc1060/main/sd_bridge.c',
 'firmware-idf/legacy-jc1060/main/probe.c',
 'firmware-idf/legacy-jc1060/rust-core/src/controller_audio.rs',
 'firmware-idf/legacy-jc1060/rust-core/src/controller_midi.rs',
 'firmware-idf/legacy-jc1060/rust-core/src/deck_play.rs',
 'firmware-idf/legacy-jc1060/rust-core/src/deck_mix.rs',
 'firmware-idf/legacy-jc1060/rust-core/src/deck_engine.rs',
 'firmware-idf/legacy-jc1060/rust-core/src/idf_media_bridge.rs',
 'firmware-idf/legacy-jc1060/rust-core/src/idf_alloc.rs',
 'firmware-idf/legacy-jc1060/third_party/esp-usb-1.6.0/usb/src/hcd_dwc.c',
 'firmware-idf/legacy-jc1060/third_party/esp-usb-1.6.0/usb/src/usb_host.c',
 'firmware-rust/crates/ruzina-audio-runtime/src/decode.rs',
]
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
result=dict(status='AUDITED_SOURCE_NOT_NEW_RUNTIME_EVIDENCE',repo_base='7a444e32bc9cdb4cd9816ef7e579ca09c1bd3c22',idf_pin=pin,
 repository_files={p:sha(repo/p) for p in files},
 idf_files={p:sha(idf/p) for p in ['components/freertos/heap_idf.c','components/hal/esp32p4/include/hal/cache_ll.h']},
 exact_control_elf=dict(path='D:/BoardEvidence/JC1060-20261030-COM3/pack-placement-20261102/clean-control/build/ruzina_legacy_probe.elf',sha256=sha(Path('D:/BoardEvidence/JC1060-20261030-COM3/pack-placement-20261102/clean-control/build/ruzina_legacy_probe.elf'))),
 storage_bytes=dict(source_ring=65536*4,decoder=0x1b228,checkpoint=0x8ed0,checkpoint_count=3,encoded=98304),
 observations=['USB callbacks serialize in _handle_pending_ep', 'HCD NUM_BUFFERS=2 with pending/done URBs', 'HCD IRQ can start next buffer before client callback', 'Catalog PASS precedes first producer/ring initialization', 'Deck2 permanent resources lazily allocated at first LOAD', 'Mix UnsafeCell requires one USB-task caller'])
result['per_deck_payload_bytes']=sum((result['storage_bytes']['source_ring'],result['storage_bytes']['decoder'],result['storage_bytes']['checkpoint']*3,result['storage_bytes']['encoded']))
assert result['per_deck_payload_bytes']==581272
Path(__file__).with_name('source-index.json').write_text(json.dumps(result,indent=2)+'\n')
print('Indexed',len(files),'repository files and',len(result['idf_files']),'pinned IDF files')
