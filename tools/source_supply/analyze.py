"""Offline integrity and wall-counter analysis; never inferred CPU or product qualification."""
import argparse
import json
from pathlib import Path
import re
from campaign import REGISTRATION_SHA, checked, require, sha
from monitor import fault_reason

STAGES=('poll','decode','step','read','push','decode_other')
USB_STAGES=('handle_events_wall','client_tail_wall','callback_entry_gap')
HEADER=('generation','schema','seq','uptime_us','epoch','index','rate','playing','pitch',
        'supply_us','busy','frames','logical_read_bytes','invalid','saturated','max_service_gap_us')


def fields(line):
    pairs=re.findall(r'(\w+)=([^\s]+)',line)
    require(len(pairs)==len({key for key,_ in pairs}),'Duplicate telemetry fields')
    return dict(pairs)


def numbers(row, keys):
    require(all(key in row and re.fullmatch(r'-?\d+',row[key]) for key in keys),'Missing/malformed numeric fields')
    return {key:int(row[key]) for key in keys}


def metric(row):
    values=numbers(row,('calls','total_us','max_us'))
    require(all(value>=0 for value in values.values()),'Negative metric')
    require(values['max_us']<=values['total_us'],'Maximum exceeds cumulative total')
    require(values['calls']>0 or values['total_us']==values['max_us']==0,'Zero calls with time')
    return values


def monotonic(rows,keys):
    for before,after in zip(rows,rows[1:]):
        require(all(after[key]>=before[key] for key in keys),'Counter regression')


def profiles(text,playing):
    decks={1:[],2:[]}
    owners=[]
    usb={stage:[] for stage in USB_STAGES}
    current=None
    seen_header=False
    def finish():
        nonlocal current
        if current is None:return
        require(set(current['stages'])==set(STAGES),'Incomplete producer stage group')
        stages=current['stages']
        require(stages['decode']['total_us']==sum(stages[s]['total_us'] for s in STAGES[2:]),'Decode partition mismatch')
        require(stages['poll']['total_us']>=stages['decode']['total_us'],'Decode exceeds parent poll')
        decks[current['deck']].append(current)
        current=None
    for line in text.splitlines():
        if line.startswith('producer_profile '):
            finish()
            row=fields(line)
            require('unavailable' not in row,'Producer snapshot unavailable')
            header=numbers(row,HEADER+('deck',))
            require(header['deck'] in decks,'Unexpected producer deck')
            require(header['schema']==1 and header['generation']==1 and header['seq']>0 and header['seq']%2==0,'Wrong schema/generation/sequence')
            require(header['invalid']==header['saturated']==0,'Invalid/saturated producer measurements')
            require(header['playing'] in (0,1) and all(v>=0 for k,v in header.items() if k!='pitch'),'Invalid producer state')
            require(-32768<=header['pitch']<=32767 and all(v<=0xffffffff for k,v in header.items() if k not in ('pitch','uptime_us')),'Producer word bounds')
            current=dict(header,stages={})
            seen_header=True
        elif line.startswith('producer_profile_stage '):
            if not seen_header:continue  # leading partial group only
            require(current is not None,'Orphan stage after the first producer header')
            row=fields(line)
            require(numbers(row,('deck','seq'))=={key:current[key] for key in ('deck','seq')},'Stage identity mismatch')
            stage=row.get('stage')
            require(stage in STAGES and stage not in current['stages'],'Duplicate/unknown producer stage')
            current['stages'][stage]=metric(row)
        else:
            if line.startswith('producer_owner '):
                finish()
                row=fields(line)
                require('unavailable' not in row,'Owner snapshot unavailable')
                owner=numbers(row,('generation','seq','uptime_us','turns','wall_us','polls_us','other_including_idle_us','max_turn_us','invalid','saturated'))
                require(owner['generation']==1 and owner['seq']>0 and owner['seq']%2==0,'Owner identity mismatch')
                require(owner['invalid']==owner['saturated']==0 and all(v>=0 for v in owner.values()),'Invalid owner measurement')
                require(owner['wall_us']==owner['polls_us']+owner['other_including_idle_us'],'Owner partition mismatch')
                require(owner['max_turn_us']<=owner['wall_us'],'Owner maximum exceeds total')
                owners.append(owner)
            elif line.startswith('producer_usb_service '):
                finish()
                row=fields(line)
                stage=row.get('stage')
                require(stage in usb,'Unknown USB service stage')
                identity=numbers(row,('generation','invalid','saturated'))
                require(identity==dict(generation=1,invalid=0,saturated=0),'Invalid USB service identity/measurement')
                converted=metric({'calls':row.get('count',''),'total_us':row.get('total_us',''),'max_us':row.get('max_us','')})
                usb[stage].append(converted)
    finish()
    intervals={}
    for deck,rows in decks.items():
        require(len(rows)>=2,'Need two complete producer snapshots per deck')
        monotonic(rows,('busy','frames','logical_read_bytes','max_service_gap_us'))
        stable=[]
        for before,after in zip(rows,rows[1:]):
            dt=after['uptime_us']-before['uptime_us']
            require(0<dt<=10_000_000 and after['seq']>before['seq'],'Stale/non-advancing producer snapshot')
            for stage in STAGES:monotonic([before['stages'][stage],after['stages'][stage]],('calls','total_us','max_us'))
            keys=('epoch','index','rate','playing','pitch')
            if before['playing']==playing[deck-1] and all(before[k]==after[k] for k in keys):
                if playing[deck-1]:require(before['epoch']>0 and before['rate']>0 and before['index']!=0xffffffff,'Playing without source')
                frame_delta=after['frames']-before['frames']
                stage_delta={s:{k:after['stages'][s][k]-before['stages'][s][k] for k in ('calls','total_us')} for s in STAGES}
                stable.append(dict(start_seq=before['seq'],end_seq=after['seq'],source_elapsed_us=dt,
                    epoch=after['epoch'],index=after['index'],rate=after['rate'],pitch=after['pitch'],playing=after['playing'],
                    frames=frame_delta,logical_read_bytes=after['logical_read_bytes']-before['logical_read_bytes'],
                    produced_frames_per_second=frame_delta*1_000_000/dt,
                    required_source_frames_per_second=before['rate']*max(1,10_000+before['pitch'])/10_000 if playing[deck-1] else 0,
                    observed_supply_us=[before['supply_us'],after['supply_us']],stages=stage_delta))
        require(stable,'No adjacent stable requested-state producer interval')
        intervals[str(deck)]=stable
    require(len(owners)>=2,'Missing owner snapshots')
    monotonic(owners,('turns','wall_us','polls_us','other_including_idle_us','max_turn_us'))
    for before,after in zip(owners,owners[1:]):
        require(0<after['uptime_us']-before['uptime_us']<=10_000_000 and after['seq']>before['seq'],'Stale owner snapshot')
    for rows in usb.values():
        require(len(rows)>=2 and rows[-1]['calls']>rows[0]['calls'],'Missing/non-advancing USB service observations')
        monotonic(rows,('calls','total_us','max_us'))
    require(len({len(rows) for rows in usb.values()})==1,'Incomplete USB service group')
    return dict(producer_samples={str(k):len(v) for k,v in decks.items()},stable_intervals=intervals,
        sample_time_bounds={**{str(k):[v[0]['uptime_us'],v[-1]['uptime_us']] for k,v in decks.items()},
                            'owner':[owners[0]['uptime_us'],owners[-1]['uptime_us']]},
        owner_first=owners[0],owner_last=owners[-1],usb_service={stage:dict(first=rows[0],last=rows[-1]) for stage,rows in usb.items()},
        wall_time_includes_wait_preemption=True,cross_core_snapshots_asynchronous=True,maxima_are_not_subtracted=True)


def analyze(folder,root,kind,playing):
    reg=json.loads(checked(root/'campaign-registration.json',REGISTRATION_SHA))
    r=json.loads((folder/'result.json').read_text())
    raw=(folder/'run.log').read_bytes()
    require(r['app_sha256']==reg['transitions']['candidate']['app_sha256'] and r['elf_sha256']==reg['transitions']['candidate']['elf_sha256'],'Wrong image')
    require(sha(raw)==r['raw_sha256'] and not r['fatal_markers'],'Capture hash/fatal mismatch')
    require(r.get('capture_complete') and not r['connection_interrupted'],'Incomplete connected evidence')
    if kind=='cold':
        window=(folder/'boot-window.log').read_bytes()
        require(sha(window)==r['boot_window_sha256'] and raw[r['boot_window_offset']:]==window,'Boot-window drift')
        require(re.findall(rb'reset_reason=(\d+)',window)==[b'1'] and b'usb_uac stream=STARTED' in window,'Missing physical cold/UAC boot')
        identities=re.findall(rb'ELF file SHA256:\s+([0-9a-fA-F]{8,64})',window)
        require(len(identities)==1 and r['elf_sha256'].startswith(identities[0].decode().lower()),'Wrong boot ELF identity')
        preparation=list(re.finditer(rb'resource_prepare state=(PREPARING|READY)\b',window))
        require([m[1] for m in preparation]==[b'PREPARING',b'READY'] and
                preparation[-1].start()<window.index(b'usb_uac stream=STARTED'),'Resource readiness ordering missing/invalid')
        require(r['post_boot_seconds']>=45,'Short cold observation')
    else:
        anchor=json.loads((root/'cold-1/result.json').read_text())
        anchor_summary=json.loads((root/'cold-1/summary.json').read_text())
        anchor_raw=(root/'cold-1/run.log').read_bytes()
        require(anchor_summary['continuation_pass'] and anchor_summary['raw_sha256']==sha(anchor_raw)==anchor['raw_sha256'],'Missing valid cold anchor')
        require(anchor['elf_sha256']==r['elf_sha256'],'Cold/image mismatch')
        require(analyze(root/'cold-1',root,'cold',(0,0))['continuation_pass'],'Cold anchor no longer validates')
        require(r.get('requires_matching_cold_identity') is True,'Unanchored capture type')
        require(b'ELF file SHA256:' not in raw and b'reset_reason=' not in raw,'Unexpected boot')
        window=raw
    require(r['window']==kind and r['status']=='CAPTURED_NOT_QUALIFIED','Wrong capture window/status')
    require(r['connected_capture_seconds']>=(60 if kind=='idle' else (45 if kind=='cold' else 30)),'Short window')
    # Preserve/hash original bytes; a capture bound may cut one final line.
    # Complete headers still require every stage, even when a final stage is cut.
    complete,separator,tail=window.rpartition(b'\n')
    parsed_window=complete+separator
    require(parsed_window,'No complete telemetry lines')
    text=parsed_window.decode('utf-8',errors='replace')
    limits=reg['measurement']['resources']
    stopped=fault_reason(parsed_window,limits) or r.get('stopped_for_fault')
    integrity=profiles(text,playing)
    phases={stage:[numbers(fields(line),('count','max_us','gt_3000','ge_6000','failures','generation'))
        for line in text.splitlines() if line.startswith('uac_fill_phase phase='+stage+' ')]
        for stage in ('initial_prefill','recovery_prefill','callback')}
    require(all(phases.values()),'Missing fill phase observations')
    callbacks=phases['callback']
    require(callbacks[-1]['count']>callbacks[0]['count'],'No callback progress')
    if kind=='cold':require(callbacks[-1]['count']>=7000,'Cold callback bound not met')
    if kind=='idle':require(callbacks[-1]['count']-callbacks[0]['count']>=9000,'Idle callback bound not met')
    resources=[numbers(fields(line),('usb_stack_free','logger_stack_free','internal_min','psram_min','internal_largest','psram_largest'))
        for line in text.splitlines() if line.startswith('engine_resources ')]
    require(len(resources)>=2,'Missing resource snapshots')
    loads=[numbers(fields(line),('stack_free',)) for line in text.splitlines() if line.startswith('rust_load ')]
    require(loads,'Missing SD-client reserve evidence')
    usb=[numbers(fields(line),('transfers','iso_errors','packet_errors','submit_errors')) for line in text.splitlines() if line.startswith('usb_uac_stats ')]
    require(len(usb)>=2 and usb[-1]['transfers']>usb[0]['transfers'],'Missing USB progress')
    mixes=[line for line in text.splitlines() if line.startswith('deck_mix_stats ')]
    require(len(mixes)>=2,'Missing mix snapshots')
    engines=[numbers(fields(line),('d1_flags','d2_flags')) for line in text.splitlines() if line.startswith('deck_engine_stats ')]
    require(len(engines)>=2,'Missing engine snapshots')
    if kind in ('cold','idle'):require(all(row['d1_flags']==row['d2_flags']==0 for row in engines),'Loaded/playing deck in idle admission')
    else:require(any(tuple(int(bool(row[k]&2)) for k in ('d1_flags','d2_flags'))==playing for row in engines),'Requested playback state never observed by engine')
    drops=[numbers(fields(line),('log_drops',))['log_drops'] for line in text.splitlines() if line.startswith('producer_water ')]
    require(len(drops)>=2 and all(value==0 for value in drops),'Missing/dropped logger evidence')
    heartbeats=[numbers(fields(line),('uptime_us',))['uptime_us'] for line in text.splitlines() if line.startswith('heartbeat=')]
    require(len(heartbeats)>=2 and heartbeats==sorted(heartbeats),'Missing/non-monotonic capture time anchors')
    for first,last in integrity['sample_time_bounds'].values():
        require(heartbeats[0]-10_000_000<=first<=heartbeats[0]+10_000_000 and
                heartbeats[-1]-10_000_000<=last<=heartbeats[-1]+10_000_000,'Stale snapshots at capture edges')
    maximum=max(row['max_us'] for rows in phases.values() for row in rows)
    return dict(status='SOURCE_DIAGNOSTIC_OBSERVATION_NOT_PRODUCT_QUALIFIED',raw_sha256=sha(raw),kind=kind,
        parsed_window_sha256=sha(parsed_window),unterminated_tail_bytes=len(tail),
        app_sha256=r['app_sha256'],elf_sha256=r['elf_sha256'],integrity=integrity,stop_reason=stopped,
        continuation_pass=stopped is None,headroom_target_pass=maximum<=3000 and all(row['gt_3000']==0 for rows in phases.values() for row in rows),
        all_fill_max_us=maximum,callback_first=callbacks[0],callback_last=callbacks[-1],
        resource_minima={key:min(row[key] for row in resources) for key in resources[0]},sd_client_stack_min=min(row['stack_free'] for row in loads),
        usb_first=usb[0],usb_last=usb[-1],mix_first=mixes[0],mix_last=mixes[-1],physical_acceptance='Record owner separately')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--root',type=Path,required=True)
    p.add_argument('--capture',required=True)
    p.add_argument('--kind',choices=('cold','idle','functional'),required=True)
    p.add_argument('--playing',choices=('0,0','1,0','0,1','1,1'),required=True)
    args=p.parse_args()
    folder=(args.root/args.capture).resolve()
    require(folder.is_relative_to(args.root.resolve()),'Capture escapes evidence root')
    output=folder/'summary.json'
    require(not output.exists(),'Summary already exists')
    result=analyze(folder,args.root,args.kind,tuple(map(int,args.playing.split(','))))
    output.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({k:v for k,v in result.items() if k!='integrity'},indent=2))


if __name__=='__main__':main()
