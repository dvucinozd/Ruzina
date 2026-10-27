# Ruzina: code review, stanje razvoja i plan popravaka

Datum: 2026-10-27. Predmet je aktualni lokalni i remote kod nakon nastavka
razvoja s drugim modelom. Ploče su još u dolasku.

Za provedbu nalaza koristi
[detaljan implementacijski plan](2026-10-27-ruzina-fix-implementation-plan.md):
mali zadaci, unaprijed određene politike, regresije i kriteriji završetka.
Plan ne znači da su popravci već implementirani.

Naknadna provedba: [execution log faze A](2026-10-27-astra-implementation-log.md)
bilježi lokalne popravke, regresije i preostale gateove. Nalazi ispod ostaju
izvorni snapshot pregledanog maina.

## Zaključak

Zajednička Rust logika i simulator znatno su napredovali od handoffa.
Velik dio RP-01..18 sada ima implementaciju i host testove. Ipak, prije
integracije na ploču treba popraviti devet nalaza niže. Šest od tih nalaza
potvrđeno je izvršivim reprodukcijama; ostali su potvrđeni čitanjem aktualnog
koda i dokumentacije.

Guition firmware još nije cjelovit DJ uređaj: board selektori pokreću scheduler
i opcionalnu dijagnostiku. Produktni owneri ostaju iza compile-only
`integration-check`, a fizički USB audio, DAC, display/touch, C6 i trajni
settings/OTA adapteri nisu povezani u prihvaćen runtime. Dolazak ploče otvara
bring-up; ne zatvara preostali softverski rad.

## Pregledani identitet i remote

- Remote: `https://github.com/dvucinozd/Ruzina.git` — projekt je preimenovan.
- Pregledani remote `main`: `4a8c660183910b0950ae7ddabce9e8b395a2f10d`,
  merge PR-a #53. SHA potvrđen fetchom i ponovnim `git ls-remote`.
- Prethodni handoff: `a04b0aa468e8b607820f056093b0f9ea41834e08`.
  Od njega su dodana 63 commita, uključujući merge commitove PR-ova #23–#53.
- Aktivni postojeći worktree:
  `C:\Users\Daniel\.codex\worktrees\m1-guition-bsp\M1_PCB`,
  grana `ogranak/rp12-web-loops-mixer`, HEAD `77654bf`.
  Njegov sadržaj prije ovog izvještaja identičan je pregledanom remote mainu.
- Primarni `D:\AI\M1_PCB` ostaje na starijem `main` / `0f2fb14`,
  65 commitova iza remotea. Ima postojeće untracked `artwork/` i neobično
  imenovanu datoteku. Nisu brisani, stageani niti prepisani.
- Nema otvorenih PR-ova ni GitHub issues u trenutku pregleda.
  Sve remote razvojne grane obuhvaćene su mainom osim
  `codex/native-apta-consumer` / `fd7ebd5`.
  Izvorna APTA grana ostaje izdvojena i OFF za Guition/default.
- U ovom pregledu nisu napravljeni merge, push, cleanup grana ni izmjene
  produkcijskog koda. Izvještaj je lokalni artefakt za provedbu popravaka.

## Dokazi i ograničenja pregleda

Aktualni commit ima uspješne remote postupke:

- [Rust host foundation](https://github.com/dvucinozd/Ruzina/actions/runs/40237557343):
  format, Clippy, workspace testovi, simulator, APTA granice i opcionalni testovi,
  runtime owner testovi te UI goldens imaju success.
- [Rust ESP32-P4 compile gate](https://github.com/dvucinozd/Ruzina/actions/runs/40237557280):
  feature matrica, obje board konfiguracije i diagnostic paketi imaju success.

Neovisno ponovljeno u izoliranom WSL checkoutu na točnom main SHA:

```sh
cd firmware-rust
cargo test --locked --workspace
cargo check --locked -p ruzina-uac -p ruzina-web -p ruzina-ota \
  -p ruzina-settings --target riscv32imafc-unknown-none-elf
python3 tools/check_docs.py
```

Rezultat: **911 passed, 0 failed, 0 ignored**, uključujući doc-test sažetke;
četiri nova contract cratea prolaze P4 target check; dokumentacijski checker
prije dodavanja izvještaja potvrđuje 72 poveznice. Zeleni link checker provjerava
postojanje odredišta, ne istinitost teksta.

Dodatne reprodukcije pokrenute su odvojeno od neizmijenjene baseline suite.
Njihov očekivani neuspjeh pokazuje rupe u postojećoj pokrivenosti; nisu
predstavljene kao prolazni regresijski testovi.

CodeRabbit CLI 0.9.1: **7 issues: 2 major, 5 minor**, kroz dva dovršena pregleda.
Prvi obuhvaća `a04b0aa..e15c562` prije preimenovanja, drugi
`b905771..4a8c660` pod `firmware-rust`. Svi prijavljeni problemi provjereni
su ponovno na aktualnim `ruzina-*` putanjama. Oba završavaju s
`review_completed` i exit 0. Cijeli prvotni raspon bio je odbijen zbog
292 datoteke / limita 150, a dva terminalska pokušaja prekinuta su SIGHUP-om;
uspješni ponovljeni pregledi izvršeni su u pozadini. Preimenovanje nije zaseban
CodeRabbit pregled; provjereni su aktualni nazivi, reference, build i testovi.

Devet stavki u nastavku konsolidira dvije USB prijave u jednu i dodaje
nalaze vlastite provjere. Prioriteti P1/P2/P3 su procjena ovog pregleda,
ne doslovno preslikavanje kategorija CodeRabbita.

Nisu ponovljeni cijeli lokalni Clippy/UI/board buildovi jer za točan SHA postoje
provjereni uspješni CI koraci. Nisu provedeni fuzzing/sanitizer kampanja, stvarni
Rekordbox import s novom playlistom, slušanje, flash, USB/I2S timing, OTA na
ploči ili soak. Niti jedan hardware PASS nije izveden iz host/CI rezultata.

## Nalazi i predloženi popravci

### R01 — P1: UAC parser može paničariti na neispravnom descriptoru

Lokacija: `firmware-rust/crates/ruzina-uac/src/descriptor.rs:226-242,264-265`.
Izvor: CodeRabbit (dvije major prijave) i neovisna reprodukcija.

Dvije odvojene putanje prelaze granice:
1. CS_ENDPOINT s duljinom 3 prolazi uvjet `len >= 3`, pa se čita `d[3]`.
2. Dva FORMAT_TYPE descriptora u istom alternateu svaki zasebno zadovoljavaju
   MAX_RATES, ali se `current.rate_count` zbraja i deveti zapis izlazi iz
   osmeročlanog polja.

Reprodukcija nad aktualnim kodom:
`three_byte_cs_endpoint: panic=true` i
`duplicate_format_type: panic=true`; panic lokacije 265 i 239.
Firmware panic handler radi software reset. UAC adapter još nije aktivan na
ploči, ali parser se mora popraviti prije njegova uključivanja.

Popravak: nedovoljno dugačke class descriptore vratiti kao
`DescriptorError::Malformed`; pratiti već viđen format/general descriptor,
odbiti duplikate i braniti kumulativnu granicu svakog polja. Samo promjena
uvjeta na `>= 4` uklanja prvi panic, ali može tiho ignorirati neispravan
descriptor, protivno dokumentiranom ugovoru.

Prihvat: obje reprodukcije vraćaju Err bez panica; truncation na svakom offsetu,
ponovljeni descriptori, MAX_RATES/MAX_FORMATS rubovi i corpus/fuzz no-panic
provjera; neispravna audio funkcija ne ruši zasebno MIDI prepoznavanje.

### R02 — P1: Web lista može povezati staru pjesmu s novom generacijom

Lokacija: `firmware-rust/crates/ruzina-web/web/app.js:122-159,183-185`.
Izvor: vlastiti pregled i Node VM reprodukcija stvarnog app.js s kontroliranim
DOM/fetch adapterom; nije tvrdnja o browser E2E testu.

Status odmah ažurira globalni `libraryRevision`, a stari redovi ostaju
klikabilni dok traje `loadLibrary()`. LOAD handler uzima ID iz starog reda i
generaciju iz nove globalne varijable. Time se zaobilazi smisao serverove
zaštite od stare liste. Na novom katalogu s istim lokalnim track ID-em može
se zatražiti druga pjesma. Zasebno, kasni odgovor starog `loadLibrary()`
može nakon nove liste ponovno objaviti stare redove.

Reprodukcije:
- red generacije 10 šalje `/api/load?track_key=42&generation=11&deck=1`;
- odgovor generacije 20 dovršen nakon 21 vraća naslov `Old volume track`.

Popravak: objavljivati jedinstveni snapshot `{boot, revision, tracks}`;
LOAD closure mora nositi generaciju tog snapshota. Pri promjeni medija
onemogućiti stare LOAD tipke, tokenom ili AbortControllerom povući prethodni
fetch, prije objave provjeriti trenutni token/identitet. Razdvojiti traženu i
uspješno učitanu reviziju: trenutačni neuspjeli fetch također se ne ponavlja
dok se globalna revizija ponovno ne promijeni.

Prihvat: zamjena A→B s ponovljenim track ID-em, klik tijekom fetcha, B odgovor
prije A, odspajanje, reboot s istim brojem revizije i privremeni HTTP error.
Stari red nikad se ne šalje s identitetom novog kataloga.

### R03 — P2: Slint highlight i LOAD mogu pokazivati različitu pjesmu

Lokacija: `firmware-rust/apps/desktop-simulator/src/main.rs:251-255`;
`session.rs:697-721` i `library_load`.
Izvor: CodeRabbit i neovisni test nad simulator Sessionom.

Na AllTracks prikazu highlight koristi `session.selected`, dok touch LOAD
koristi cursor LibraryBrowsera. Web load mijenja prvi, a ne drugi.
Sličan problem nastaje nakon učitavanja iz playliste i povratka BACK.

Reprodukcija nakon Web loada druge pjesme:
`UI highlight=1, browser selection=Some(0)`.
Sljedeći touch LOAD učita `01 / Grid study · 120 BPM`, iako je označeno
`02 / Offset beats · 128 BPM`.

Popravak: isti LibraryBrowser cursor mora određivati highlight i touch LOAD
kad je medij prisutan. Ako Web load treba pomicati izbor, to napraviti preko
eksplicitne browser operacije, a ne drugog izvora istine.

Prihvat: Web LOAD→touch LOAD i playlist LOAD→BACK→LOAD uvijek učitavaju
označeni red; ažurirati odgovarajući UI smoke/golden ako se izgled promijeni.

### R04 — P2: Web socket blokira Slint timer

Lokacija: `firmware-rust/apps/desktop-simulator/src/web.rs:138-173`;
poziv `web.poll` iz 50 ms UI timera u `main.rs`.
Izvor: CodeRabbit i stvarna loopback socket reprodukcija.

Svaka prihvaćena veza obrađuje se sinkrono na UI threadu. Read timeout je po
pojedinom readu, nema apsolutnog roka ni ograničenja broja veza po pollu.
Postupno slanje bajtova može produžiti obradu do minuta; write nema timeout.
Jedna spojena veza koja ne pošalje ništa zadržala je `poll` **258 ms**.

Popravak: ograničen nonblocking state machine ili zaseban I/O worker s
ograničenim redovima prema Sessionu; ukupni read/write deadline, broj veza,
broj bajtova i budžet rada po UI ticku. Sam deadline od 250 ms poboljšava
najgori slučaj, ali i dalje prelazi UI interval od 50 ms.

Prihvat: preconnect, spori header/body, odbijeni body, više paralelnih veza
i klijent koji ne čita odgovor ne zaustavljaju transport, UI ni druge zahtjeve.
Body admission izvršiti čim je header potpun, prije čekanja odbijenog bodyja.

### R05 — P2: Zauzeti backend troši sequence i gubi retry kontrolu

Lokacija: `firmware-rust/crates/ruzina-web/src/server.rs:222-240`;
`ordering.rs:53-89`.
Izvor: vlastiti pregled i dodatni HTTP adapter test.

`LatestValueGate::admit` upiše sequence prije `backend.control`.
Ako backend vrati Busy, klijent dobije 503. Isti retry kasnije se smatra
superseded, dobije 200 OK i ne šalje se backendu.

Reprodukcija: client 7 / seq 12 / volume 1000, prvo Busy, zatim slobodan
backend. Retry dobije 200, a `fake.controls.len()` ostaje 0 umjesto 1.

Popravak: razlikovati viđeni, pending i prihvaćeni sequence ili koristiti
rezervaciju/commit; Busy mora ostaviti identičan posljednji zahtjev retryable,
uz odbijanje starijih vrijednosti nakon uspješno prihvaćene novije.
Ne uklanjati cijelu zaštitu od reorderinga.

Prihvat: Busy→isti retry, Busy→noviji uspjeh→stari retry, dvije stranice,
različite kontrole i wraparound. 200 za ponovno prihvaćen zahtjev ne smije
značiti tiho izgubljenu vrijednost. Simulator sada uvijek vraća Applied;
problem je u zajedničkom ugovoru za budući ograničeni produktni queue.

### R06 — P2: M2.5 import zaobilazi zaštitu mrežnog zapisa

Lokacija: `firmware-rust/crates/ruzina-settings/src/m25.rs:123-145`.
Izvor: CodeRabbit i dvije neovisne regresije.

Import provjerava samo status općih settingsa. Direktni `commit_network`
može zamijeniti postojeću samostalno spremljenu mrežu ili zapis
`NewerPreserved`, iako redovna `set_network` putanja to odbija.

Reprodukcije:
- spremljena mreža `Chosen` postaje `Legacy`;
- validni network format 2 / NewerPreserved postaje format 1 /
  `Stored { sequence: 2 }`.

Popravak: migracijsku politiku primijeniti na oba neovisna zapisa.
Sačuvati noviji format i postojeći drugačiji korisnički network.
Nastavak prekinutog importa smije prepoznati identičan već spremljen network;
izričito definirati odbija li se cijeli import ili se preskače samo network.

Prihvat: opći settings Defaults/Rejected uz network Defaults/Stored/Newer,
različite i jednake vrijednosti, power cut između network i settings commita,
ponovni boot. Provjeriti i efektivni zapis nakon boota, ne samo RAM.

### R07 — P2: Neuspjelo MP3 index čitanje nema predviđeni backoff

Lokacija: `firmware-rust/apps/m1-firmware/src/deck_audio_source.rs:474-488,876-889`.
Izvor: CodeRabbit i provjera aktualnog control flowa; nije mjereno na ploči.

`scan_frame_index` vraća `true` i kad `read_at` vrati nula bajtova, bez
napretka indeksa. Pozivatelj tada preskače 1 ms čekanja. Kod ponavljane I/O
greške uz valjan media lease i pun PCM queue owner neprestano šalje
seek/read pokušaje zajedničkom USB0 brokeru i može opteretiti drugi deck.

Popravak: uspjeh znači stvarni napredak, ne samo pokušaj; za nula bajtova
vratiti false. Dodati ograničen retry/backoff i odustajanje od opcionalnog
indeksa nakon trajne greške, zadržavajući ispravni playback fallback.

Prihvat: fake filesystem s trajnim i povremenim read errorom; ograničen broj
scan zahtjeva po vremenu, uredan PCM napredak drugog decka, prioritet
seek/load/cancel i nema lažne oznake kompletne duration.

### R08 — P2: Dokumentacija usmjerava sljedeći razvoj na stare zadatke

Lokacije: `README.md:42-52`, `docs/DEVELOPMENT.md:7-14`,
`docs/HANDOFF.md:3-5,40-58,213-236`,
`firmware-rust/apps/desktop-simulator/README.md:71`.
Izvor: vlastiti pregled; simulator README i CodeRabbit.

README još navodi title index/S3CP/CUE/playliste kao sljedeće neodrađene
korake. DEVELOPMENT kaže da nema HTTP servera, dok simulator ima Web Remote.
HANDOFF govori da je sav planirani host rad gotov, ali istodobno upućuje na
staru continuation granu i navodi da se od starog 315-test audita mijenjala
samo dokumentacija. Simulator README tvrdi da Web UI još ne postoji.
Dio parity plana već je ažuriran, pa dokumenti daju proturječne upute.

Popravak: README, DEVELOPMENT i HANDOFF uskladiti s jednom tablicom
host/shared implemented — simulator wired — target compiled — board verified.
Stare audite označiti kao povijesne, zadržati njihove originalne SHA i rezultate,
a tekući nastavak vezati uz main i konkretne otvorene zadatke.

Prihvat: agent koji čita samo početne dokumente neće ponovno implementirati
RP-01 niti zaključiti da je ostalo samo čekanje ploče. Link checker i ručna
provjera tvrdnji moraju proći.

### R09 — P3: Web PITCH fader ne prati objavljeno stanje

Lokacija: `firmware-rust/crates/ruzina-web/web/app.js:84-99`.
Izvor: čitanje aktualnog renderera.

`renderDeck` osvježava tekst pitcha, a popis slidera obuhvaća volume/EQ/filter,
ali ne i pitch. Nova stranica ili promjena iz druge kontrole može pokazivati
drugačiji pitch od položaja fadera; sljedeći pomak šalje vrijednost iz tog
neusklađenog položaja.

Popravak: u status uključiti točnu raw vrijednost fadera i range, ili koristiti
jednoznačnu inverznu pretvorbu zajedničkog pitch ugovora. Osvježavati slider
kad ga korisnik ne povlači. Ne pretvarati beat-sync/nudge offset naslijepo u
položaj fizičkog fadera.

Prihvat: početno spajanje na netrivialan pitch, promjena iz Slinta/drugog
klijenta, centar i krajnje vrijednosti; prikaz i sljedeći poslani događaj slažu se.

## Do gdje smo došli

| Cjelina | Potvrđeno u kodu/testovima | Što još nedostaje |
|---|---|---|
| RP-01/02 Rekordbox/profili | Title index 17, završetak PDB lanca, DAT/EXT rubovi; S3CP v2/v3/v4, neovisne golden datoteke i upload transaction | Stvarni export s playlistama; fizički USB import; HTTP profile upload i board storage |
| RP-03/04/05/15/18 transport | CUE hold/PLAY commit, VINYL/CDJ, jog/nudge, LOAD LOCK, memory/loop/source cues, reconnect fencing; simulator veze | Popravci selectiona; produktni runtime i stvarni controller/LED/seek hot-plug scenariji |
| RP-06/07/08 biblioteka/media | Playlist hierarchy, PWV4/markers, cover/JPEG/fallback, bounded MP3 index i sample-exact seek testovi | Board cover/display pipeline, real-export playlist corpus, I/O backoff i stvarni worst-case latency |
| RP-11 settings | A/B CRC zapisi, commit prije RAM objave, network validacija, eksplicitni donor import; simulator RAM store | R06; target partition adapter, real persistence i power-cut testovi; RAM simulator ne preživljava restart procesa |
| RP-12 Web/C6 | Host guard/order/status ugovori, HTTP core, stvarni loopback simulator server, Web deck/library/loop/mixer stranica | R02/R04/R05/R09; profile/OTA/resources/config/diagnostic handleri, C6 transport, SoftAP/mDNS/DNS, target backend/meters |
| RP-13 OTA | Potpis, manifest/hash/version/board/project binding, fake A/B slotovi i rollback/readiness state machine | Pravi flash/otadata/partition adapter, TLS CA/hostname, upload/download povezivanje, signing i target failure evidence |
| RP-09/10/14/16/17 audio/platform | UAC descriptor/packet/epoch/teardown, 48→44.1 kHz, MAIN change, odvojeni PFL gain, MT regression i resource ugovori | R01; UAC/I2S/DMA adapteri, sink receipts, clock/cache/heap/stack budžeti, stvarni output i slušanje |
| Guition BSP | Odvojeni profili, min rev 302, scheduler/USB Serial JTAG logovi, first-boot i memory diagnostic paketi | Fizička identifikacija i svaki periferijski driver, allocator/DMA kvalifikacija, aktiviranje produktnih ownera |
| RP-19/20, APTA, recorder | Granice scopea su zapisane | Dodatni controlleri/absolute PFL i DJ Link ostaju zasebni; native APTA i recorder nisu preduvjet za prvi bring-up |

Zapisani raniji real-export rezultat od 189 pjesama nije ponovno reproduciran
u ovom pregledu: nalazi se u HANDOFF-u i ne obuhvaća export s playlistama.
Ne prenositi ga na fizički USB import ili cijelu M2.5 parity kvalifikaciju.

## Redoslijed prije dolaska ploče

1. **Popravci pouzdanosti, zasebni reviewable commitovi:** prvo R01 i R02,
   zatim R03–R07; R09 uz Web batch. Svaki počinje regresijom koja pada na
   pregledanom SHA, a prolazi nakon popravka. Nakon toga relevantne host,
   runtime-owner, UI i target provjere.
2. **Aktualizirati razvojni ugovor:** R08; objaviti jedinstveni status po RP-u,
   označiti simulator demo i fake backend granice, te zamijeniti tvrdnju
   “sav host rad gotov” konkretnim preostalim zadacima.
3. **Zatvoriti još moguće host integracije:** spojiti profile upload,
   settings/network config, OTA upload/pull i resource/diagnostic API preko
   fake adaptera; testirati streaming, cancel/retry, power loss i admission
   kroz cijeli API→owner put. Route 501 ili postojeći trait nije dovršena
   produktna funkcija. Recorder rute zadržati izvan R1.
4. **Proširiti stvarne fixture dokaze:** legalno dostupni Rekordbox exporti
   s nested playlistama, više medija s istim lokalnim track ID-evima i
   dozvoljeni descriptor capturei planiranog kontrolera. Dodati parser
   no-panic/truncation/fuzz i frontend async regressions u CI.
5. **Pripremiti board integracijski checklist:** ownership periferija,
   resource map, neaktivni target adapter skeletoni i točni first-boot
   paketi. Povezati host ugovore u composition testove bez izmišljanja GPIO-a,
   particijskih adresa ili hardverske spremnosti.

## Redoslijed nakon dolaska ploče

1. Zabilježiti model, PCB/panel/unit oznake i opaženi silicon; napraviti i
   provjeriti factory backup i ROM recovery prije programiranja.
2. Minimalni Rust boot i logovi, pa flash/PSRAM probe i readback identitet.
   Trenutačni 64 KiB CPU-cached PSRAM test na 80 MHz ne potvrđuje cijeli
   kapacitet, production clocks ili DMA/cache koherenciju.
3. Kvalificirati stvarne memorijske regije/allocator, panel/backlight/touch,
   zatim SD napajanje i media lifecycle; zasebno za svaki model.
4. Potvrditi USB PHY/pin/host/VBUS/backfeed vlasništvo prije kontrolera i
   sticka; zatim MIDI/profili/LED, MSC/import i decode.
5. Prva puna audio putanja s jednim deckom, zatim dva decka i stress:
   controller UAC, vanjski stereo MAIN DAC, povrat na controller MAIN,
   teardown/receipts, level/latency/listening, mixed-rate MT/loop/scratch/FX.
   **Slušalice ostaju isključivo preko kontrolera** pri svim MAIN promjenama.
6. Trajni settings/Hot Cue journal, C6/Web i signed OTA s fizičkim rollbackom
   i power-cut/timeout scenarijima; završiti exact-image soak i release evidence.

Otvorena board pitanja iz postojeće matrice ostaju: JC1060 touch reset/IRQ,
JC8012 GSL3680 payload/provenance, stvarni panel, USB-FS mapping i host power.
Nije provedena nova provjera vendor materijala u ovom code reviewu.

## Lokalni trag provjera za sljedećeg agenta

Izolirani test checkout:
`/home/daniel/.cache/ruzina-review-20261027/repo` u WSL Ubuntu.
Postojeći Windows radni checkout nije dobio eksperimentalne testove.

Izvan repoa, direktorij
`C:\Users\Daniel\.codex\worktrees\m1-guition-bsp\research` sadrži
`uac-review-repro.rs`, `web-review-repro.cjs`, dodatke
`web-extra.rs`, `settings-extra.rs`, `simulator-extra.rs`,
`selection-extra.rs` i skripte `review-*-20261027.sh`.

WSL audit direktorij čuva baseline `host-tests.log`, `host-runner.log`,
`uac-repro.log`, `regressions.log`, `web-busy-regression.log`,
`settings-regressions.log`, `selection-regression.log` i dva dovršena
`coderabbit-*-retry.ndjson`. Reprodukcije namjerno potvrđuju kvar; nisu popravci.
Datoteke `*-extra.rs` služe kao polazište za formalne regresije u zasebnom
fix branchu. Popravci i njihovo objavljivanje nisu provedeni ovim izvještajem.
