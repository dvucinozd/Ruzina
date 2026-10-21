from pathlib import Path
import hashlib, importlib.util, json, subprocess, sys
root=Path('/mnt/c/Users/Daniel/.codex/worktrees/m1-guition-bsp')
source=root/'M1_PCB/firmware-rust/tools/guition_first_boot.py'
spec=importlib.util.spec_from_file_location('boot',source);boot=importlib.util.module_from_spec(spec);spec.loader.exec_module(boot)
target=Path('/home/daniel/.cache/m1-rust-review/fixture-target')
manifest=root/'research/guition-native-record/Cargo.toml'
subprocess.run(['cargo','+1.95.0','build','--offline','--manifest-path',str(manifest),'--target-dir',str(target)],check=True)
binary=target/'debug/guition-native-record-fixture'
output=root/'research/guition-memory-fixtures-20261021';output.mkdir()
results=[]
for probe in boot.PROBES:
    for board in boot.MODELS:
        package=root/f'research/guition-memory-prep-20261021/{probe}/{board}'
        value=boot.package(package)
        assert value['diagnostic_probe']==probe
        assert value['image_id'].endswith('-'+probe)
        for case in ['matching','failure','wrong-image']:
            image=value['image_id'] if case!='wrong-image' else 'WRONG-IMAGE-SOFTWARE-FIXTURE'
            raw=subprocess.check_output([str(binary),board,image,probe,'failure' if case=='failure' else 'matching'])
            file=output/f'{probe}-{board}-{case}.log';file.write_bytes(raw)
            destination=output/f'{probe}-{board}-{case}'
            command=[sys.executable,str(source),'ingest','--package',str(package),'--log',str(file),'--output',str(destination),'--unit',f'SOFTWARE_FIXTURE-{board}-{probe}','--scenario','cold-boot']
            result=subprocess.run(command,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
            assert result.returncode==(0 if case=='matching' else 2),result.stdout
            record=json.loads((destination/'result.json').read_text())
            assert record['scenario_acceptance']=='NOT_QUALIFIED'
            assert (destination/'raw.log').read_bytes()==raw
            results.append({'probe':probe,'board':board,'case':case,'exit':result.returncode,'errors':record['errors'],
                'raw_sha256':hashlib.sha256(raw).hexdigest(),'scenario_acceptance':record['scenario_acceptance']})
        symbols=subprocess.check_output(['nm','-C',str(package/'firmware.elf')],text=True)
        read_id=[line for line in symbols.splitlines() if 'platform::memory::read_id' in line]
        if probe in ('flash','memory'):
            assert len(read_id)==1,read_id
            address=int(read_id[0].split()[0],16)
            assert 0x4ff00000<=address<0x50000000,read_id
        else:assert not read_id,read_id
        results.append({'probe':probe,'board':board,'read_id_symbols':read_id,'elf_sha256':boot.digest(package/'firmware.elf')})
(output/'summary.json').write_text(json.dumps({'software_fixtures_only':True,'results':results},indent=2)+'\n')
print('All 24 actual Rust boot/service/memory fixture captures and 8 package/symbol checks PASS; hardware NOT RUN')
