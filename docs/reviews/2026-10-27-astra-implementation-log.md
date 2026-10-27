# Provedba Astrina plana

Base: `4a8c660183910b0950ae7ddabce9e8b395a2f10d`, remote provjeren fetchom.
Grana: `fix/astra-review-phase-a`. Datum: 2026-10-27.
Rad se odvija u postojećem Guition worktreeu; primarni checkout je sačuvan.
Izmjene su uncommitted dok nije drugačije navedeno.

| Task | Status | Dokaz |
|---|---|---|
| A00 | VERIFIED | Fetch, status i worktree inventar; nova fix grana iz origin/main |
| A01 | VERIFIED host | Dvije nove regresije panikama padaju prije fixa; nakon fixa 30 UAC testova prolazi; dodatni truncation test |
| A02 | VERIFIED Node | Svih pet async testova pada prije fixa, zatim prolazi; puni stvarni app.js u VM, Node 22.23.3 |
| A03 | VERIFIED host/UI | Regresija prvo pada (Some(1) umjesto Some(0)), nakon fixa prolazi; stvarni refresh/get_selection provjeren u smokeu |
| A04 | VERIFIED host | Idle socket regresija prvo pada na blokirajućem readu; fake partial I/O, deadline, EOF i connection cap testovi te realni loopback |
| A05 | VERIFIED host | Busy→retry prvo daje nula primjena; nakon fixa duplicate/Busy/newer ordering test prolazi |
| A06 | VERIFIED host | Obje nove regresije prvo padaju; sačuvani sequence, reboot vrijednosti i byte-identični noviji flash nakon fixa |
| A07 | VERIFIED host/target | Policy limit/reset test i codec_streams partial-index/playback test; firmware owner poziva policy, integration-check compile PASS |
| A08 | VERIFIED Node/host | PITCH regresija prvo pada, zatim prolazi; raw JSON/IDLE adapter, drag isolation |
| A09 | VERIFIED local gates | Završni workspace 925/0/0; Clippy, UI, target owner i oba board checka PASS; CI/browser/hardware ostaju otvoreni |
| A10 | VERIFIED docs | README, DEVELOPMENT, HANDOFF, parity/simulator dokumenti linkaju jedinstvenu status matricu; link checker PASS |

Naredbe A01: `cargo test --locked -p ruzina-uac`.
Naredbe A02: `node --test crates/ruzina-web/web/tests/app.test.cjs`.
Linux Rust native okruženje koristi postojeći WSL Rust 1.95 cache i sysroot.
Faze B/C nisu dio ovog fix batcha. APTA OFF i controller-only headphones pravila
ostaju na snazi. Nije izvršen push/merge/flash.

## Završna verifikacija

Linux/WSL Ubuntu; Rust 1.95.0; Node 22.23.3. Lokalni logovi su u
`/home/daniel/.cache/ruzina-review-20261027/astra-*.log`.
Testirani su radni izvori na gore navedenoj bazi, ne nova release slika.

| Provjera (cwd firmware-rust osim navedenog) | Rezultat |
|---|---|
| `cargo test --locked --workspace` prvi puni prolaz | 922 passed / 0 failed / 0 ignored; kasnije dodana još tri testa |
| `cargo test --locked --workspace` završni prolaz | **925 passed / 0 failed / 0 ignored**, 93 test result grupe |
| `cargo fmt --all --check` i format standalone firmware manifesta | PASS; firmware izvori dodatno formatirani preko njegova manifesta |
| `node --test crates/ruzina-web/web/tests/app.test.cjs` | 6 passed / 0 failed |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | PASS nakon popravka nedostajućeg smoke importa i clone-on-copy u novom testu |
| `cargo test --locked -p ruzina-service --features runtime-owners --test runtime_owners -- --nocapture --test-threads=1` | 10 passed |
| `cargo test --locked -p ruzina-service --features runtime-owners --test composed_failures -- --nocapture --test-threads=1` | 18 passed |
| `cargo clippy --locked -p ruzina-service --features runtime-owners --tests -- -D warnings` | PASS |
| `cargo run --locked -p desktop-simulator -- --smoke target/ui-smoke` | PASS, stvarni Slint refresh/callback/pointer/keyboard, obje geometrije |
| `python3 tools/check_ui_goldens.py target/ui-smoke` | 64 passed, postojeći goldens nisu mijenjani |
| `cargo check --locked -p ruzina-uac -p ruzina-web -p ruzina-ota -p ruzina-settings --target riscv32imafc-unknown-none-elf` | PASS |
| cwd apps/m1-firmware: `cargo +1.95.0 check --locked --features integration-check,render-graph-bringup` | PASS |
| cwd apps/m1-firmware: `cargo +1.95.0 check --locked --features board-jc1060` | PASS |
| cwd apps/m1-firmware: `cargo +1.95.0 check --locked --features board-jc8012` | PASS |
| `python3 tools/check_docs.py` | 90 passed |
| `git diff --check` | PASS, samo line-ending obavijesti |

Slika `jc1060-library-playlist.ppm` ručno je pregledana nakon software rendera:
označeni red, podaci i LOAD/BACK gumbi uredno se prikazuju. Dodana smoke
assertion provjerava da refresh nakon Web LOAD-a prikazuje LibraryBrowser cursor.

Ograničenja: Web JS je provjeren izvršavanjem stvarnog app.js u kontroliranom
VM/DOM harnessu i HTTP preko realnog loopback socketa. Interaktivni Web browser
smoke nije proveden. Nije pokrenut remote CI za ove neobjavljene izmjene niti
cijeli P4 feature/build/package matrix. Fizički timing, broker fairness, USB,
DAC, C6, flash i OTA acceptance ostaju NOT RUN do dolaska ploča.
Za A07 nije dodan test koji izvršava async firmware owner na hostu: policy i
stvarni SourceDecoder/index put testirani su zajedno na hostu, owner wiring
je provjeren čitanjem i target compileom. To ne mjeri board latenciju.

Oba board checka imaju postojeća upozorenja u nepovezanim owner importima;
exit code je 0. Nisu maskirana novim allow atributima.

Sljedeći zadatak: B01 contract matrica
preostalih HTTP ruta; zatim B02 bounded body transport. Sva implementacija
trenutačno je lokalna i uncommitted.
