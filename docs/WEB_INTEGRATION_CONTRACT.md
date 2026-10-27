# Host HTTP management contract (phase B)

2026-10-27. Base `4a8c660`; local phase A is preserved on
`fix/astra-review-phase-a`. This is a host contract, not device qualification.

All API routes use existing Host/Origin/X-DDJ-Control admission. Credentials
in queries are refused. Duplicate identity/length headers are malformed.
Transfer-Encoding remains unsupported. Each connection closes after one response.
Response bytes are bounded by the caller buffer; overflow yields 500.

| Method / route | Body and public result | Owner and persistence |
|---|---|---|
| GET /api/ota/config | no body; ssid, url, password_set; never password | SettingsOwner::network |
| POST /api/ota/config | strict JSON <=512 B: ssid, url, password OR keep_password:true OR clear:true | SettingsOwner::set_network/clear_network; 200 only after durable commit |
| GET /api/controller-profiles | no body; bounded profiles array: id/version/vid/pid; simulator scope | ProfileRegistry from validated installed blobs |
| POST /api/controller-profile | 32..16384 B binary; X-DDJ-Profile-ID, optional X-DDJ-Profile-Overwrite 0/1 | ProfileScanBuffers::install then registry rescan; valid S3CP only |
| GET /api/firmware | no body; running/candidate state, simulated flag; boot_confirmed false for pending image | OTA install + BootHealth model; no automatic confirmation |
| POST /api/ota/p4 | 212..4194492 B signed DDJOTA1; application/octet-stream | Incremental local-policy installer, inactive SlotFlash; candidate only after signature/hash/descriptor checks |
| GET /api/resources | no body; simulated scope and measurement availability | bounded resource snapshot; unavailable heaps null, no invented device measurements |
| GET /api/diagnostic-log | no body; bounded redacted owner snapshot, units/scope stated | Service snapshot; no raw arbitrary log strings or request URLs |

JSON config rejects unknown/duplicate keys, trailing content, ambiguous types
and escapes rather than interpreting a partial object. Explicit empty password
means open network; omission requires keep_password:true. Empty body never clears.
clear:true is exclusive. Existing newer record refuses mutation (409).

Management endpoints accept no query parameters, including encoded credentials.
Profile size admission preserves the existing M2.5 400 refusal; config/OTA size
limits use 413. The demo Session deliberately returns 503 for OTA until a real
trust/project/slot composition is configured; tests use an explicit fake device.

Management uploads use a single server-issued lease. Begin validates the entire
head before consuming bytes. Only the current lease may write/finish/cancel;
a stale cancel must not affect its successor. Second upload =>503 until first
finishes or cancels. Input chunks <=4096 B, exact Content-Length, no pipelining.
Short body/EOF, extra bytes, absolute 30 s body deadline or I/O error cancel the
lease; uncommitted config/profile bytes are discarded. OTA aborts inactive slot
and resumes playback if it had stopped. Completed upload cannot accept more data.
Headers have the existing 2 s deadline; writes have 2 s. Rejected bodies are not
drained. A slot failure =>500, malformed input/signature/hash =>400, existing
profile/newer settings =>409, unavailable adapter/busy =>503, unsupported =>501.

Profile activation is separate from durable install: rescan uses the existing
controller epoch/activation ticket and does not mark a profile active until the
normal initialization path completes. A bad upload never replaces the active
profile. Fake host storage is explicitly per process; power-cut/recovery tests
must reconstruct owners from persisted fake state.

OTA preserves existing policy: signed local service rollback may install an
older image; pull requires a newer signed version matching the offer. Production
trust keys/project IDs are never inferred from test fixtures. Host tests use the
existing test key and ruzina-m1-test project. TLS, C6 network, real flash/rollback
and physical audio stop/resume remain OPEN. No pull-trigger endpoint is added;
pull remains controlled by its existing owner API/fake transport tests.

Recorder routes remain 501 and outside R1. Real controller captures/exports can
only enter fixtures with provenance and permission; no private music is added.
Existing fixture manifests remain authoritative; synthetic cases are labelled.

Implementation/validation progress: [phase B execution log](reviews/2026-10-27-phase-b-log.md).
