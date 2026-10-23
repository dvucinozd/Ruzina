# Signed OTA test bundles (CC0)

Synthetic `.ddjota` bundles in the M2.5 layout (188-byte header, ECDSA P-256
over the first 124 bytes, raw `r || s`), followed by a byte pattern that starts
with an ESP32-P4 image header. They are not firmware and contain no
third-party code.

They are signed with a throwaway test key. Only its public point,
`test_key.sec1`, is committed; the private key was discarded. These bundles
therefore never verify against the M2.5 release key, and the tests check that.
`generate.py` (Python 3, `cryptography`) regenerates every bundle and the
public point together with a fresh key.

- `newer.ddjota`: `M2.5-4-g1a2b3c4d`, project `pajoniiir-m1-test`, 4219-byte image.
- `newer_high_s.ddjota`: the same header with the high-S form of the
  signature, which M2.5's mbedTLS verifier also accepts.
- `older.ddjota`: `M2.4-7-g0badc0d`, for the newer-only pull and signed local
  rollback cases.
- `other_project.ddjota`: validly signed for project `other-board`, so identity
  is refused even though the signature is good.
