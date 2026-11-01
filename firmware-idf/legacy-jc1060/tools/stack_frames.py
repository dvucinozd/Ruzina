"""Prologue stack-frame sizes from RISC-V objdump output.

The Rust client tasks have fixed internal stacks. An async state machine that
inlines nested futures can reserve tens of KiB in one poll frame; the hardware
stack guard only reports that on the device. This reads each function's
prologue (`addi sp,sp,-N`, or `lui`/`addi` into a register then
`sub sp,sp,reg`) up to the first branch, call or return.
"""
import functools
import re

# One Rust frame above this cannot be a plain local; it is an inlined future
# or buffer temporary that belongs in the heap or a static.
RUST_FRAME_LIMIT = 24576


def frame_sizes(disassembly):
    sizes, func, regs, done = {}, None, {}, True
    for line in disassembly.splitlines():
        header = re.match(r'^[0-9a-f]+ <(.+)>:$', line)
        if header:
            func, regs, done = header[1], {}, False
            continue
        fields = line.split('\t')
        if done or func is None or len(fields) < 2:
            continue
        op = fields[1].strip()
        args = fields[2].split('#')[0].replace(' ', '').split(',') if len(fields) > 2 else []
        if op == 'addi' and args[:2] == ['sp', 'sp'] and int(args[2]) < 0:
            sizes[func] = sizes.get(func, 0) - int(args[2])
        elif op == 'lui':
            value = int(args[1], 0) << 12
            regs[args[0]] = value - (1 << 32) if value >= 1 << 31 else value
        elif op == 'addi' and args[0] == args[1] and args[0] in regs:
            regs[args[0]] += int(args[2])
        elif op == 'sub' and args[:2] == ['sp', 'sp'] and args[2] in regs:
            if regs[args[2]] > 0:
                sizes[func] = sizes.get(func, 0) + regs[args[2]]
            done = True
        elif op in ('ret', 'j', 'jal', 'jalr', 'jr', 'mret') or op.startswith(('b', 'c.j')):
            done = True
    return sizes


def oversized_rust_frames(disassembly, limit=RUST_FRAME_LIMIT):
    return {name: size for name, size in frame_sizes(disassembly).items()
            if size > limit and name.startswith(('_ZN', 'ruzina_'))}


def deepest_stack(disassembly, root):
    """Worst-case stack of `root`: its frame plus the deepest chain of direct
    calls (`jal`/`jalr` with a symbol, and tail `j`). Indirect calls through
    pointers are not followed and recursion is cut, so this is an estimate
    that only grows the frames already measured; it returns (bytes, path)."""
    sizes, calls, func = frame_sizes(disassembly), {}, None
    for line in disassembly.splitlines():
        header = re.match(r'^[0-9a-f]+ <(.+)>:$', line)
        if header:
            func = header[1]
            calls.setdefault(func, set())
            continue
        target = re.search(r'\t(?:jalr?|j)\s.*<([^>+]+)>', line)
        if func and target and target[1] != func:
            calls[func].add(target[1])

    @functools.lru_cache(None)
    def walk(name, depth):
        best = (0, ())
        if depth < 64:
            for callee in calls.get(name, ()):
                best = max(best, walk(callee, depth + 1))
        return sizes.get(name, 0) + best[0], (name,) + best[1]

    total, path = walk(root, 0)
    return total, list(path)
