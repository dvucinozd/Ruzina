# Offline paired-source layout preparation

This generation is deliberately **offline only**, with no execution/serial
adapter. It prepares the matched-reset producer control and opt3 images from
the exact 44-plan executed prefix and validates rollback of the larger image
erase spans. Old source-supply writers/contracts/bindings remain byte-unchanged
and do not authorize the proposed plans.

```sh
python tools/source_compare/prepare.py --root <fresh-external-directory>
python -m unittest discover -s tools/source_compare/tests -v
```

Real-file preparation requires the pinned IDF Python/esptool image parser,
four retained factory copies, all historical payload/restore files, exact
previous execution captures and the three clean reset packages. Unit tests use
synthetic layouts; a unit-test PASS is not real-file preparation or device proof.

The script pins source/app/ELF/manifest and history identities, replays all
executed payloads and saved pre-write spans, and checks the final prior whole
flash against retained full readbacks. The earliest historical plans have no
embedded prefix list; their hash-linked predecessor chain is still verified.
Every present embedded chain must match exactly. No link or history count is
inferred from a repeating full-flash hash.

Proposed control/return/opt3/return predecessors are 44/45/46/47. They are not
executed entries. Both diagnostic images are larger than s07, so exact returns
use complete saved baseline sector snapshots, with their own explicit payload
kind. A normal smaller-image rewrite leaves stale bytes outside its erase span.
Saved predecessor bytes are validated independently of their recorded hashes;
each proposed return is replayed back to the complete original state.

The script removes runnable templates and marks every plan
`LAYOUT_ONLY_UNBOUND_NOT_EXECUTABLE`. A new reviewed adapter must enforce
snapshot-return semantics, exact ancestry, source identities and device
readback gates before calling the unchanged shared no-reset writer. New passive
capture and separately bound boot authorities are still required. No proposal
or old binding is an execution permit. See the
[frozen runbook](../../docs/reviews/2026-11-03-source-compare-runbook.md).
