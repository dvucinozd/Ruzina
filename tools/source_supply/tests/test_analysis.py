"""Synthetic counter transcripts validate parsing; no fabricated device evidence."""
from pathlib import Path
import json
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from analyze import profiles,STAGES,USB_STAGES,analyze
from campaign import sha
from monitor import fault_reason,CaptureCoverage,capture_complete
from campaign import validate_candidate_manifest,SOURCE,program,_shared


def transcript(playing=(1,0),samples=(1,2)):
    lines=[]
    for i in samples:
        for deck in (1,2):
            lines.append(f'producer_profile deck={deck} generation=1 schema=1 seq={i*2} uptime_us={i*6000000} epoch=1 index={deck} rate=44100 playing={playing[deck-1]} pitch=0 supply_us=1000000 busy={i*10} frames={i*264600} logical_read_bytes={i*1000} invalid=0 saturated=0 max_service_gap_us=100')
            for stage,total in zip(STAGES,(100,80,20,20,20,20)):
                lines.append(f'producer_profile_stage deck={deck} seq={i*2} stage={stage} calls={i*10} total_us={i*total} max_us=10')
            lines.append(f'heartbeat={i} uptime_us={i*6000000}')  # other tasks may interleave logger lines
        lines.append(f'producer_owner generation=1 seq={i*2} uptime_us={i*6000000} turns={i*20} wall_us={i*400} polls_us={i*200} other_including_idle_us={i*200} max_turn_us=20 invalid=0 saturated=0')
        for stage in USB_STAGES:
            lines.append(f'producer_usb_service generation=1 stage={stage} count={i*100} total_us={i*1000} max_us=10 invalid=0 saturated=0')
    return '\n'.join(lines)+'\n'


class AnalysisTests(unittest.TestCase):
    def test_full_capture_identity_resources_and_anchor_are_checked(self):
        limits=dict(usb_stack_min=2048,logger_stack_min=1024,internal_heap_min=65536,psram_heap_min=8388608,
            observed_internal_largest_min=8192,observed_psram_largest_min=262144,sd_stack_min=8192)
        app,elf='b'*64,'a'*64
        reg=dict(transitions={'candidate':dict(app_sha256=app,elf_sha256=elf)},measurement=dict(resources=limits))
        reports=[]
        for i in range(1,9):
            reports.append(transcript((0,0),[i]))
            reports.extend([
                f'usb_uac_stats transfers={i*1000} iso_errors=0 packet_errors=0 submit_errors=0\n',
                'deck_mix_stats underruns=0/0\n','deck_engine_stats d1_flags=0 d2_flags=0\n',
                'engine_resources usb_stack_free=4000 logger_stack_free=2000 internal_min=100000 psram_min=10000000 internal_largest=20000 psram_largest=1000000\n',
                'rust_load event=13 stack_free=50000\n','producer_water log_drops=0\n'])
            for stage in ('initial_prefill','recovery_prefill','callback'):
                reports.append(f'uac_fill_phase phase={stage} generation=1 count={i*1000} max_us=2000 gt_3000=0 ge_6000=0 failures=0\n')
        raw=('ELF file SHA256: '+elf+'\nreset_reason=1\nresource_prepare state=PREPARING\nresource_prepare state=READY\nusb_uac stream=STARTED\n'+''.join(reports)).encode()
        result=dict(app_sha256=app,elf_sha256=elf,raw_sha256=sha(raw),fatal_markers=[],capture_complete=True,
            connection_interrupted=False,boot_window_sha256=sha(raw),boot_window_offset=0,post_boot_seconds=45,
            window='cold',status='CAPTURED_NOT_QUALIFIED',connected_capture_seconds=45,stopped_for_fault=None)
        with tempfile.TemporaryDirectory(prefix='source-analysis-synthetic-') as temporary:
            root=Path(temporary);folder=root/'cold-1';folder.mkdir()
            (folder/'run.log').write_bytes(raw);(folder/'boot-window.log').write_bytes(raw)
            (folder/'result.json').write_text(json.dumps(result))
            with patch('analyze.checked',return_value=json.dumps(reg).encode()):
                self.assertTrue(analyze(folder,root,'cold',(0,0))['continuation_pass'])
                cut=raw+b'usb_uac_stats transfers='
                (folder/'run.log').write_bytes(cut);(folder/'boot-window.log').write_bytes(cut)
                (folder/'result.json').write_text(json.dumps({**result,'raw_sha256':sha(cut),'boot_window_sha256':sha(cut)}))
                trimmed=analyze(folder,root,'cold',(0,0))
                self.assertTrue(trimmed['continuation_pass'])
                self.assertEqual(trimmed['raw_sha256'],sha(cut))
                self.assertEqual(trimmed['unterminated_tail_bytes'],len(cut)-len(raw))
                (folder/'run.log').write_bytes(raw);(folder/'boot-window.log').write_bytes(raw)
                (folder/'result.json').write_text(json.dumps({**result,'capture_complete':False}))
                with self.assertRaises(ValueError):analyze(folder,root,'cold',(0,0))
                (folder/'result.json').write_text(json.dumps(result))
                (folder/'run.log').write_bytes(raw+b'edited')
                with self.assertRaises(ValueError):analyze(folder,root,'cold',(0,0))

    def test_complete_partitions_and_stable_source_rates(self):
        data=profiles(transcript(),(1,0))
        self.assertEqual(data['producer_samples'],{'1':2,'2':2})
        interval=data['stable_intervals']['1'][0]
        self.assertEqual(interval['produced_frames_per_second'],44100)
        self.assertEqual(interval['required_source_frames_per_second'],44100)
        self.assertEqual(interval['stages']['decode']['total_us'],80)

    def test_malformed_stale_and_incoherent_evidence_is_denied(self):
        source=transcript()
        broken=[
            source.replace('stage=read calls=10 total_us=20','stage=read calls=10 total_us=19',1),
            source.replace('stage=step calls=10 total_us=20 max_us=10\n','',1),
            source[:source.rfind('producer_profile_stage')],
            source.replace('seq=4 uptime_us=12000000','seq=2 uptime_us=12000000',1),
            source.replace('uptime_us=12000000','uptime_us=22000000'),
            source.replace('invalid=0','invalid=1',1),
            source.replace('saturated=0','saturated=1',1),
            source.replace('stage=read calls=10','stage=read calls=0',1),
            source.replace('seq=4 stage=push','seq=2 stage=push',1),
            source.replace('other_including_idle_us=200','other_including_idle_us=199',1),
            source.replace('producer_owner generation=1','producer_owner unavailable=1 generation=1',1),
            source.replace('stage=client_tail_wall count=200 total_us=2000 max_us=10 invalid=0 saturated=0','',1),
        ]
        for data in broken:
            with self.subTest(data=data[:80]),self.assertRaises(ValueError):profiles(data,(1,0))

    def test_mixed_state_is_not_steady_throughput(self):
        source=transcript().replace('playing=1','playing=0',1)
        with self.assertRaises(ValueError):profiles(source,(1,0))

    def test_only_leading_orphan_stages_may_be_skipped(self):
        leading='producer_profile_stage deck=2 seq=0 stage=read calls=1 total_us=1 max_us=1\n'
        profiles(leading+transcript(),(1,0))
        with self.assertRaises(ValueError):profiles(transcript()+leading,(1,0))

    def test_candidate_flags_are_exact_and_write_guard_unchanged(self):
        manifest=dict(source_commit=SOURCE,dirty_status='',variant='deck-engine',producer_profile=True,uac_profile=False,uac_pack_iram=False)
        validate_candidate_manifest(manifest)
        for key,value in [('producer_profile',False),('producer_profile',1),('uac_profile',True),('uac_pack_iram',True),('source_commit','old')]:
            with self.subTest(key=key),self.assertRaises(ValueError):validate_candidate_manifest({**manifest,key:value})
        self.assertIs(program,_shared.program)
        self.assertNotIn('serial',sys.modules)
        self.assertNotIn('esptool',sys.modules)

    def test_fault_detection_stops_on_first_reported_error(self):
        limits=dict(usb_stack_min=2048,logger_stack_min=1024,internal_heap_min=65536,psram_heap_min=8388608,
            observed_internal_largest_min=8192,observed_psram_largest_min=262144,sd_stack_min=8192)
        self.assertIsNone(fault_reason(b'usb_uac_stats iso_errors=0 packet_errors=0 submit_errors=0\n',limits))
        for line in ['usb_uac_stats iso_errors=0 packet_errors=1 submit_errors=0',
                     'deck_mix_stats underruns=1/0','uac_fill_phase generation=1 max_us=6000 ge_6000=1 failures=0',
                     'producer_profile unavailable=1','producer_profile invalid=1',
                     'producer_water log_drops=1','rust_load event=6 stack_free=50000']:
            with self.subTest(line=line):self.assertIsNotNone(fault_reason((line+'\n').encode(),limits))
        coverage=CaptureCoverage('cold',600,0);coverage.boot(10);coverage.disconnected(20)
        self.assertFalse(capture_complete('cold',coverage.seconds(100),600,coverage.seconds(100),False,coverage.interrupted))


if __name__=='__main__':unittest.main()
