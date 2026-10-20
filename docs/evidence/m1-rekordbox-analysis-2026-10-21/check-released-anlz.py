from pathlib import Path
import json, subprocess
root = Path(r'C:\Users\Daniel\.codex\worktrees\m1-guition-bsp\research')
out = root/'m22-anlz-reference'
out.mkdir(exist_ok=True)
baseline = '2c2ec32c253d368765123d7bbf8d37389b790b55'
repo = Path('D:/Documents/DDJ-FFL4')
for name, relative in [('rekordbox_anlz.c','firmware/main-deck-p4/components/library/rekordbox_anlz.c'),('rekordbox_anlz.h','firmware/main-deck-p4/components/library/include/rekordbox_anlz.h')]:
    (out/name).write_bytes(subprocess.check_output(['git','show',f'{baseline}:{relative}'],cwd=repo))
(out/'source.json').write_text(json.dumps({'repository':'https://github.com/dvucinozd/Pajoniiir','tag':'M2.2','commit':baseline,'scope':'read-only comparison; C implementation not copied into Rust product'},indent=2)+'\n')
print(baseline)
