# Separate accepted-s07 software boot action

This implements the action requested by the [owner recovery policy](../RECOVERY_POLICY.md),
separate from the unchanged no-reset writer. No device access or reset was
performed while developing it. Current board remains accepted s07/history 42.
Neither the consumed 40/41 plans nor a prior tool binding authorize this code.

[`software_boot.py`](../../tools/source_supply/software_boot.py) defaults to
offline validation of a new hash-pinned boot policy and the retained return
proof. A future 42/43 campaign needs `REGISTERED_S07_SOFTWARE_BOOT`, exact unit,
rev103/MAC, native minimum 302, accepted app/ELF and expected whole-flash state.
It requires return predecessor count 43/history afterward 44 and the new
return plan/registration hashes. Existing 41-predecessor proof is rejected.

The offline proof validator rehashes both complete 16 MiB files and all three
return spans, checks fresh programmer identity/security results, and compares
span bytes with the full readback. It also hashes the exact 768176-byte accepted
app and checks its ELF header. No stored success label substitutes for bytes.
The current restored whole hash is `28830827...`; future registration must
reconstruct and verify it against the full executed chain before using this
fixed policy. Changed images need separate qualification and policy.

Execution additionally needs a separate `REVIEWED_S07_SOFTWARE_BOOT_TOOLS`
binding covering this file, its campaign dependency and unchanged shared
guard. It checks pinned IDF Python, esptool 5.6.0, esp-pylib 1.2.0 and pyserial
3.5. The exact low-level `esp_pylib/serial_reset.py` SHA-256 is
`52d636ed8eff0855cf9e8c44deb1c5392ea642be153f9dee2931ad334d474b88`.
No new boot policy/binding has yet been registered.

After proof/binding/toolchain checks, the action verifies COM3 USB identity,
sets DTR/RTS false before opening, and rechecks enumeration identity. It calls
the pinned low-level `hard_reset` once with hold/release delays of 0.2 seconds
and flow_control=false. It uses no ROM run/flash command, stub download,
watchdog reset, esptool retry strategy or swallowed-warning fallback. This is
a USB control-line action, not a physical power cycle. Its applicability to
this exact return is not physically measured by the offline tests.

The fresh result distinguishes attempted and completed pulse calls, close
failures and unknown boot state. Even on success it records
`BOOT_PULSE_ISSUED_APPLICATION_NOT_OBSERVED` and
`application_running=NOT_OBSERVED`. It requests no cold, listening, Pause/CUE
or owner acceptance sequence. A failed pulse is recorded and not retried.
The already accepted image recovery closes with automatic readback; this
separate action records the attempt to start it without inventing a running
application. The campaign orchestrator must close every monitor first and
retain the return proof and boot result.

Seven new synthetic tests cover policy/native/ancestry constraints, exactly
one pulse, unsupported controls, partial open, identity changes, close failure,
output reuse, old binding/proof rejection and full-file/span/app integrity.
No synthetic hashes or fake port calls become a real execution binding. All
44 source-supply tests pass on Windows and Linux; 13 unchanged shared-guard
tests pass on Linux. The [continuous adapter](2026-11-03-continuous-adapter.md)
and this action still need successful review/CI and new registration before
any target use. The initial review attempt hit a CodeRabbit rate limit; no
review result is inferred from tests or dashboard setup.
