from pathlib import Path
import json, re, shlex, subprocess
root=Path('/mnt/c/Users/Daniel/.codex/worktrees/m1-guition-bsp')
workflow=(root/'M1_PCB/.github/workflows/rust-esp32p4.yml').read_text()
block=re.search(r"          python - <<'PY'\n(.*?)\n          PY",workflow,re.S)
exec(compile('\n'.join(line[10:] for line in block.group(1).splitlines()),'<partition validator>','exec'))
commands=re.findall(r'^        run: (cargo \+1\.95\.0 .+)$',workflow,re.M)
assert len(commands)==48,len(commands)
results=[]
for index,command in enumerate(commands,1):
    print(f'{index}/{len(commands)} {command}',flush=True)
    with (root/f'research/guition-p4-{index:02}.log').open('w') as log:
        result=subprocess.run(shlex.split(command),stdout=log,stderr=subprocess.STDOUT)
    results.append({'command':command,'exit':result.returncode,'log':f'guition-p4-{index:02}.log'})
    (root/'research/guition-matrix-summary.json').write_text(json.dumps({'partition_validator':'PASS','results':results},indent=2)+'\n')
    assert result.returncode==0,command
