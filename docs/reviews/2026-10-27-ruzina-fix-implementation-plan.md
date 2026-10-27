# Ruzina: izvršivi plan popravaka nakon code reviewa

Datum: 2026-10-27. Status: **PLAN; popravci još nisu implementirani.**

Naknadno: faza A implementirana je lokalno; rezultate i otvorene gateove vidi u
[execution logu](2026-10-27-astra-implementation-log.md). Ostatak ovog dokumenta
čuva izvornu uputu i početnu checklistu; nije samostalna potvrda završetka.

Izvor: [detaljni code review](2026-10-27-ruzina-code-review.md).
Pregledani remote main: `4a8c660183910b0950ae7ddabce9e8b395a2f10d`.
Remote: `https://github.com/dvucinozd/Ruzina.git`.

Ovaj dokument je uputa za model koji treba male, jednoznačne zadatke.
Provesti prvo fazu A, zadatak po zadatak. Faza B opisuje sljedeće host
integracije; faza C ovisi o stvarnoj ploči. Dovršena faza A ne znači da je
firmware spreman za nastup, flash ili release.

## 1. Pravila koja se ne mijenjaju

- Ciljne ploče su Guition `JC1060P470C_I_W_Y` i `JC8012P4A1C_I_W_Y`.
  Korisnik navodi silicon v3.2; potvrditi ga fizičkim readoutom po dolasku.
- Slušalice/PFL uvijek idu preko kontrolera. Spreman vanjski DAC može
  preuzeti samo stereo MAIN; zatim slijedi fallback na controller MAIN,
  a bez raspoloživog izlaza mute. Detekcija prisutnosti nije dokaz spremnosti.
- `codex/native-apta-consumer` ostaje neintegriran. APTA ostaje OFF u
  default/Guition proizvodu; postojeće izolirane testove sačuvati.
- Recorder ostaje izvan R1. Ne implementirati ga usput.
- Ne vraćati vlastitu PCB/KiCad arhitekturu ni njezin pinout.
- Ne uklanjati testove, ne smanjivati provjere kako bi CI postao zelen,
  ne ažurirati goldens bez pregleda stvarne promjene slike.
- Compile PASS, host PASS, simulator PASS i hardware PASS odvojene su tvrdnje.
- Ne dodavati stvarne particijske adrese, GPIO-e, touch payload ili sigurnosne
  ključeve na temelju pretpostavke. Nepoznat hardware detalj ostaje otvoren.

## 2. Siguran početak i radni postupak

### A00 — identificiraj bazu, sačuvaj tuđi rad

Trenutačno postoji odgovarajući worktree:
`C:\Users\Daniel\.codex\worktrees\m1-guition-bsp\M1_PCB`.
Na datum plana grana je `ogranak/rp12-web-loops-mixer`, HEAD `77654bf`,
a tracked sadržaj odgovara navedenom main SHA. Review i ovaj plan su novi
lokalni dokumenti. Primarni `D:\AI\M1_PCB` je stariji i sadrži korisnikove
untracked datoteke; ne koristiti ga kao izvor aktualnog koda.

U odabranom worktreeu pokreni zasebno:

```powershell
git status --short --branch
git worktree list
git remote -v
git fetch origin --prune
git rev-parse HEAD
git rev-parse origin/main
git diff --stat HEAD origin/main
git log --oneline -8 origin/main
```

Ako je remote napredovao, najprije provjeri vrijedi li svaki nalaz i zabilježi
novu bazu. Ne resetiraj checkout i ne prepisuj korisnikove izmjene.
Za implementaciju otvori zasebnu fix granu iz provjerene baze; postojeće
dokumente prenesi bez gubitka. Ako baza više nije pregledani kod, ne tvrdi da
su brojevi testova ili reprodukcije iz reviewa dokaz za novi SHA.

Sve relativne putanje u nastavku računaju se od korijena repoa. Cargo/Node
naredbe, osim posebno navedenih, izvršavaju se iz `firmware-rust/`.
Rust verziju i Linux native ovisnosti preuzmi iz
[host workflowa](../../.github/workflows/rust-host.yml).
Za target koristi [P4 workflow](../../.github/workflows/rust-esp32p4.yml).
Na Windowsu se može koristiti WSL; Git operacije nad Windows managed
worktreeom izvršavaj Windows Gitom. Ne koristi audit cache kao čistu bazu:
`/home/daniel/.cache/ruzina-review-20261027/repo` sadrži namjerno dodane
regresije iz pregleda.

Za svaki zadatak:

1. Pročitaj navedene funkcije i postojeće testove.
2. Dodaj test koji zahtijeva ispravno ponašanje i potvrdi da pada prije fixa.
   Test koji potvrđuje da se bug dogodio nije završna regresija.
3. Implementiraj samo taj zadatak i potrebne izmjene njegovih call siteova.
4. Pokreni ciljane provjere. Zapiši naredbu, exit code i ishod.
5. Pregledaj diff. U dnevniku označi zadatak završenim tek nakon provjere.
6. Ako je commit autoriziran, napravi mali commit s testom i fixom zajedno.
   Predloženi naslovi ispod nisu nalog za push, merge ili release.

## 3. Faza A — obvezni popravci prije daljnje integracije

| Red | Zadatak | Nalaz | Ovisi o | Granica završetka |
|---|---|---|---|---|
| 1 | A01 UAC parser | R01 / P1 | A00 | Malformed input vraća Err, bez panike |
| 2 | A02 Web library snapshot | R02 / P1 | A00 | Stari red ne šalje novi identitet |
| 3 | A03 Library selection | R03 / P2 | A00 | Highlight i touch LOAD koriste isti cursor |
| 4 | A04 Neblokirajući WebPort | R04 / P2 | A00 | Spori klijent ne blokira UI timer |
| 5 | A05 Busy i ordering | R05 / P2 | A00 | 503 ne troši sequence |
| 6 | A06 M2.5 network migration | R06 / P2 | A00 | Postojeći/noviji network record je sačuvan |
| 7 | A07 MP3 index retry | R07 / P2 | A00 | Failed scan nema neograničen read loop |
| 8 | A08 Web pitch | R09 / P3 | A02 test harness | Slider prikazuje raw fader |
| 9 | A09 Završna verifikacija | svi | A01–A08 | Host, UI i target rezultati zabilježeni |
| 10 | A10 Dokumentacija | R08 / P2 | A09 | Status odgovara kodu i dokazima |

Slijedi red tablice. Ne spajaj sve u jedan veliki refactor.

### A01 — odbij neispravne UAC deskriptore bez panike

**Datoteka:** `firmware-rust/crates/ruzina-uac/src/descriptor.rs`.
Ulaz: `parse_playback_formats`; pregledaj i `PlaybackFormat::complete` te
postojeće descriptor fixture helper funkcije.

**Problem:** CS_ENDPOINT duljine 3 čita `d[3]`. Dva FORMAT_TYPE deskriptora
unutar istog alternata akumuliraju više od osam rateova u polje od osam.

**Koraci:**

1. Dodaj dvije regresije u postojeći test modul. Sastavi ispravno configuration
   zaglavlje `[9,2,total_lo,total_hi,1,1,0,0x80,50]`, iza njega AS interface
   `[9,4,1,1,1,1,2,0,0]`. U prvoj dodaj `[3,0x25,1]`.
   `wTotalLength` mora odgovarati stvarnoj duljini. Očekuj `Err`, bez panike.
2. Za drugu dodaj dva Type I deskriptora: svaki počinje
   `[32,0x24,2,1,4,2,16,8]`, iza toga osam puta `[0x80,0xBB,0]` (48000 Hz).
   Ukupna duljina svakog je 32. Očekuj `Err`; ne prihvaćaj parcijalni rezultat.
3. Prije svakog čitanja polja provjeri duljinu prepoznatog deskriptora.
   Za CS_ENDPOINT ne dopusti pristup indeksu 3 pri `len < 4`; poznati,
   prekratki deskriptor mora dati `Malformed { offset }`, a ne biti prešućen.
   Pregledaj i guardove za AS_GENERAL i AS_FORMAT_TYPE: prepoznati krnji
   deskriptori ne smiju pasti u granu koja ignorira nepoznati subtype.
4. Prati `seen_format_type` po alternatu. Resetiraj ga na svakom INTERFACE.
   Drugi FORMAT_TYPE u istom AS alternatu odbij kao Malformed prije pisanja.
   Dodatno obrani svako upisivanje ratea provjerom kapaciteta; koristi postojeći
   `TooManyRates` za prekoračenje. Ne oslanjaj se samo na broj iz novog headera.
5. Sačuvaj ponašanje za podržani PCM, odvojene alternate i nepodržane formate.
   Ne dodavati UAC2, resampling ili novu descriptor selekciju u ovaj commit.

**Testovi:** oba gore navedena slučaja; 0/1/8/9 discrete rateova prema postojećem
ugovoru; dvije valjane različite alternate; nepoznati subtype; prefixi valjanog
configuration buffera bez panike. Za prefix test razlikuj slučaj nepromijenjenog
total lengtha od svjesno prepisanog total lengtha: potonji može biti valjana,
kraća konfiguracija i ne mora uvijek dati Err. Sačuvaj MIDI odvajanje.

**Provjere:** `cargo test --locked -p ruzina-uac` i target check iz odjeljka 4.
**Gotovo:** produkcijski parser odbija obje reprodukcije; nema catch_unwind
u produkciji ni tihog truncationa rateova.
**Commit:** `fix(uac): reject malformed and duplicate format descriptors`.

### A02 — koherentan Web popis i zaštita od zakašnjelih odgovora

**Datoteke:** `firmware-rust/crates/ruzina-web/web/app.js`, novi
`firmware-rust/crates/ruzina-web/web/tests/app.test.cjs`,
`.github/workflows/rust-host.yml`.

**Odluka:** identitet prikazanog popisa je par `(boot, library_revision)`.
Novi status odmah invalidira stare LOAD gumbe. Popis se objavljuje tek nakon
uspješnog preuzimanja svih stranica za taj identitet. API parametar `generation`
i dalje je revision; ovaj fix ne uvodi novi HTTP protokol.

**A02a — test harness:** koristi `node:test`, `node:assert/strict`, `node:vm`,
`node:fs` iz Node standardne biblioteke. U VM učitaj stvarni app.js, s fake
DOM-om, kontroliranim fetch promiseovima, crypto vrijednošću i fake timerima.
Ne kopiraj algoritam aplikacije u test. Bootstrapping `bindControls`/`pollStatus`
može odraditi fake DOM i fetch; ne uklanjaj dijelove izvora regexom kako bi
regresija prošla. Dopušteno je izdvojiti malu eksplicitnu start funkciju uz
očuvano automatsko pokretanje u browseru. Node 22 koristi se kao test runtime;
u CI ga postavi preko `actions/setup-node@v4`, `node-version: '22'`.

**A02b — stanje i algoritam:**

1. Zamijeni značenje globalnog `libraryRevision` s odvojenim stanjima:
   `desiredIdentity`, `loadedSnapshot`, `libraryRequestToken`, aktivan zahtjev,
   `retryAt` i retry attempt. Snapshot sadrži vlastite tracks i identitet.
2. Nakon prihvaćenog statusa usporedi oba dijela identiteta. Na promjenu povećaj
   token, poništi loaded snapshot, isprazni/ponovno iscrtaj listu te poništi
   retry stare generacije. Null revision prikaži kao No media, bez fetchanja.
3. `loadLibrary` na početku kopira identitet i token. U svakom nastavku nakon
   `await` provjeri da su još aktualni. Isto vrijedi za catch/finally: stari
   zahtjev ne smije promijeniti poruku, in-flight flag ili retry novoga.
4. Svaka stranica mora imati očekivanu generation. `next` mora napredovati
   ili biti null; ponovljeni offset je pogreška, ne nova beskonačna petlja.
   Na mismatch odbaci cijeli pokušaj i zakaži retry dok je identitet aktualan.
5. Nakon zadnje stranice još jednom provjeri identitet i token; tek tada
   atomarno postavi loaded snapshot i renderiraj redove.
6. LOAD callback zatvara track_key i identitet toga snapshota. Prije slanja
   mora potvrditi da snapshot još odgovara aktualnom loaded i desired stanju.
   Stari callback tada radi no-op; ne sastavlja stari key s novom generacijom.
7. Za isti identitet dozvoli jedan aktivan pokušaj. Na neuspjeh čekaj 1 s,
   zatim 2 s, 4 s i najviše 5 s između daljnjih pokušaja. Retry provjeravaj
   iz postojećeg status poll ciklusa; ne stvaraj nekontrolirane timeout lance.
   Uspjeh ili promjena identiteta resetira backoff. Status bez promjene
   revisiona mora ipak moći pokrenuti dospjeli retry.

**Regresije s upravljanim redoslijedom promiseova:**

- Učitaj revision 10, key 42. Prihvati status 11, zadrži list fetch. Klikni
  sačuvani callback starog reda: ne smije poslati `/api/load` ni s 10 ni s 11.
- Započni fetch za 20, zatim za 21; dovrši 21 pa 20. Vidljivi tracks i LOAD
  ostaju iz 21. Ponovi s rejected promiseom starog zahtjeva.
- Druga stranica ima drugu generation: nema parcijalne objave ni LOAD gumba.
- Prvi fetch padne; isti revision i protek retry vremena dovrše popis.
- Boot A/revision 5 → boot B/revision 5 ipak invalidira i ponovno učita listu.
- Uklanjanje medija dok fetch traje ostavlja prazan popis; zakašnjeli odgovor
  ga ne vraća. Ponovljeni `next` završava pokušaj, ne beskonačno fetchanje.
- Search nakon učitavanja koristi isti snapshot i njegov identitet.

Ovo štiti promjene boota koje je klijent opazio. Ne tvrditi da client-only fix
može otkriti reboot između zadnjeg statusa i POST-a; server-side boot admission
bio bi zasebna promjena protokola.

**Provjere:** `node --test crates/ruzina-web/web/tests/app.test.cjs`,
`cargo test --locked -p ruzina-web`; dodaj Node naredbu u host workflow.
**Gotovo:** testovi kontroliraju async raspored bez stvarnog sleepa i izvršavaju
stvarni app.js; browser list/load smoke je također zabilježen.
**Commit:** `fix(web): bind library loads to committed catalog snapshots`.

### A03 — uskladi označeni red i touch LOAD

**Datoteke:** `firmware-rust/apps/desktop-simulator/src/main.rs` (`refresh`),
`src/session.rs` (`library_selected`, `library_load`, `web_load`), `src/tests.rs`
unutar istog simulator direktorija; po potrebi postojeći UI smoke scenarij.

**Odluka:** dok je medij povezan, highlight uvijek prikazuje LibraryBrowser
cursor, i na AllTracks i u playlisti. Web LOAD učitava traženi deck, ali ne
pomiče korisnikov browser cursor. Demo/disconnected prikaz može koristiti
`session.selected`, uz onemogućeno učitavanje s nepovezanog medija.

**Koraci:**

1. Dodaj `Session::visible_library_selection() -> Option<usize>`: connected
   vraća `library_selected()`, disconnected vraća postojeći demo odabir.
   Koristi helper u `refresh` za `ui.set_selection`, None mapiraj na -1.
   `library_top` ostaje samo informacija o prikazu, ne drugi izvor selekcije.
2. Ne pokušavaj riješiti problem automatskim premještanjem LibraryBrowsera
   u `web_load`: to bi promijenilo dogovorenu semantiku browsanja.
3. Regresija: browser cursor ostaje na prvom tracku, Web LOAD učita drugi
   u deck 1. Visible selection mora biti prvi; sljedeći touch LOAD u deck 2
   mora učitati taj prvi, vidljivo označeni track. Ne očekivati drugi track
   samo zato što je zadnji učitan preko Weba.
4. Pokrij playlist LOAD → BACK → touch LOAD, ručno pomicanje kursora,
   prazan popis, reconnect i disconnected demo. U UI testu pozovi stvarni
   `refresh` i provjeri `get_selection`; samo unit test novog helpera nije
   dovoljan dokaz da ga UI zaista koristi.

**Provjere:** `cargo test --locked -p desktop-simulator`, UI smoke/goldens iz
odjeljka 4. **Gotovo:** prikaz i LOAD ostaju povezani nakon remote učitavanja.
**Commit:** `fix(simulator): render the active library browser selection`.

### A04 — ukloni blokirajući socket rad iz UI timera

**Datoteke:** `firmware-rust/apps/desktop-simulator/src/web.rs`, simulator
`src/tests.rs`; `main.rs` samo ako se promijeni potpis `poll`.
Pročitaj `ruzina-web/src/http.rs` i `server.rs` prije promjene transporta.

**Odluka:** ostaje single-threaded nonblocking state machine. Session i dalje
pripada UI threadu; ne uvoditi worker thread, Tokio, mutex oko Sessiona ili
novi executor. Ovo je loopback simulator adapter, ne gotov P4 HTTP driver.

**A04a — model veze:**

- Najviše 8 aktivnih veza. Najviše 2 accept pokušaja po `poll` pozivu.
- Svaki prihvaćeni TcpStream odmah `set_nonblocking(true)`.
- Stanja: Reading, Writing, Closed; read buffer ograniči na postojeći
  `http::HEAD_MAX_BYTES` (2048), response na `assets::LARGEST_BYTES + 512`.
- Reading pamti start vrijeme i duljinu; Writing buffer, offset i start.
- Apsolutni deadline Reading 2 s, Writing 2 s. Novi byte ne produljuje rok.
- Po vezi i pollu najviše jedan read do 1024 bytes ili jedan write do 4096
  bytes. Obilazak rotira početnu vezu radi pravednosti. Nema `write_all`.
- Kad je tablica puna, novoprihvaćenu vezu zatvori; ne alociraj dodatni slot.

**A04b — obrada:**

1. `poll_at(now, session)` obrađuje konačan broj koraka. Javni `poll` mu
   predaje Instant::now; test može kontrolirati vrijeme.
2. WouldBlock znači ostavi stanje za drugi tick. EOF s nepotpunim headerom
   zatvara vezu bez promjene Sessiona. Ostale I/O greške također zatvaraju.
3. Čim je header potpun, pozovi postojeći WebServer jednom. Ne čekaj
   `Content-Length` bytes: trenutačne implementirane rute ne primaju body,
   a WebServer sam vraća 413 ili 501 za ostale slučajeve.
4. Za parser grešku osim Incomplete pripremi postojeći error odgovor preko
   WebServera. Na isteku reading/writing deadlinea zatvori vezu; ne pokušavaj
   dodatni blokirajući error write. Incomplete pun header je 431, ne rast buffera.
5. Writing čuva partial-write offset; tek nakon slanja svih bytes zatvori
   socket. Značenje `poll` return broja definiraj kao potpuno poslane odgovore
   i prilagodi postojeće testove tom ugovoru.
6. Dodaj malu internu I/O apstrakciju samo ako treba za fake WouldBlock i
   short-write testove. Fake i TcpStream moraju koristiti isti state machine.

**Testovi:** idle socket uz normalan GET; klijent koji stalno šalje po byte
ipak istekne na 2 s; primatelj koji ne čita ne blokira drugi zahtjev; djelomični
read/write sačuva točne bytes; prekid u svakoj fazi oslobodi slot; >8 veza ne
povećava memorijsku granicu; velik Content-Length dobije postojeći 413/501
odmah nakon headera, bez čekanja bodyja. Odgovor i side effect smiju nastati
samo jednom. Fake I/O provjerava broj operacija, a ne brzinu CI računala.

Dodaj i realni loopback smoke s idle peerom i aktivnim GET-om. Timing rezultat
zabilježi kao smoke; strogi `<50 ms` test nije glavni deterministički gate.

**Provjere:** simulator testovi i UI smoke. **Gotovo:** nema blokirajućeg
read/write, sleepa ni neograničenog accept loopa unutar `poll`.
**Commit:** `fix(simulator): serve web requests with bounded nonblocking polling`.

### A05 — sequence potvrdi tek nakon prihvaćene naredbe

**Datoteke:** `firmware-rust/crates/ruzina-web/src/ordering.rs`, `src/server.rs`,
`tests/http_server.rs` u istom crateu.

**Odluka:** tablica pamti zadnju uspješno prihvaćenu naredbu, ne zadnju viđenu.
Busy ne mijenja ni sequence ni LRU stanje. Server obrađuje naredbe serijski;
ne uvoditi pending rezervacije za konkurentnost koja ovdje ne postoji.

**Koraci:**

1. Zamijeni mutirajući `admit` dvjema operacijama:
   `classify(&self, client, control, sequence) -> Ordering` i
   `commit_applied(&mut self, client, control, sequence)`.
2. `classify` koristi postojeći wrap-aware `newer`; nema upisa. Duplicate i
   stariji od accepted sequence ostaju Superseded. Razlika pola u32 raspona
   ostaje ne-novija kao dosad.
3. U serveru prvo classify, zatim backend.control. Samo Applied commitira
   watermark i LRU te daje 200. Busy daje 503 bez commita. Nekontinuirane
   naredbe i pozivi bez client/seq zadržavaju postojeći put.
4. LRU clock povećava se na commit; kapacitet ostaje 32. Zamijeni stare
   testove pozivima classify+commit tamo gdje simuliraju stvarno prihvaćanje.

**Regresije:** fake backend prvo Busy, zatim Applied za isti client/control/seq
10: prvi 503, drugi 200 i točno jedna primjena, treći duplicate 200 bez nove
primjene. Busy 10 → Applied 11 → retry 10 ne primjenjuje 10. Busy 11 nakon
Applied 9 smije dopustiti 10 jer 11 nikad nije prihvaćen. Različiti controls i
clienti, wrap MAX→0 i LRU eviction moraju ostati pokriveni.

**Provjere:** `cargo test --locked -p ruzina-web`.
**Gotovo:** HTTP regresija broji stvarne backend primjene; nije samo test gatea.
**Commit:** `fix(web): commit control ordering after backend acceptance`.

### A06 — migracija ne smije prepisati network record

**Datoteke:** `firmware-rust/crates/ruzina-settings/src/m25.rs`, postojeći fake
flash helperi u `src/store.rs` samo ako nedostaje način usporedbe zapisa.
Pročitaj `owner.rs` i sve `M25NetworkImport` call siteove prije promjene enum-a.

**Dogovorena politika:**

| Settings status | Network status | Ishod |
|---|---|---|
| Stored | bilo koji | AlreadyMigrated, nula upisa |
| NewerPreserved | bilo koji | NewerRecordPreserved, nula upisa |
| Defaults/Rejected | NewerPreserved | NewerRecordPreserved, nula upisa u oba recorda |
| Defaults/Rejected | Stored | Sačuvaj mrežu, importiraj samo product settings |
| Defaults/Rejected | Defaults/Rejected | Validiraj legacy mrežu, zatim postojeći import |

**Koraci:**

1. Napravi network status preflight prije prvog commit_network ili
   commit_settings. Noviji network blokira cijeli import, čak i kad legacy
   nema network podatke; to je namjerno konzervativna politika.
2. Dodaj `M25NetworkImport::PreservedExisting`. Za Stored network vrati taj
   rezultat i ne zapisuj mrežu, čak ni kad su legacy bytes identični.
   Invalid legacy network također ne smije nadjačati postojeći valjani zapis.
3. Defaults/Rejected mreža zadržava Absent/Imported/Rejected ponašanje.
   Network se piše prije settingsa; settings zapis ostaje marker dovršenog
   importa. Ne mijenjati format recorda ni serializer.
4. Nastavak nakon prekida između network i settings upisa sada daje
   PreservedExisting, a ne Imported. Svjesno ažuriraj taj postojeći test;
   provjeri da nema drugog mrežnog upisa i da završni settings odgovara legacyju.
5. Ažuriraj module doc komentar i exhaustive matcheve za novi enum variant.

**Regresije:** postojeći SSID/URL/password ostaju nakon importa i reboota;
noviji CRC-valjan network record ostaje byte-identičan i bez upisa settingsa;
isti Stored record ne povećava sequence; prekinuti import se može dovršiti;
invalid legacy mreža na praznom storeu i dalje je Rejected; AlreadyMigrated
je idempotentan. U logovima i assertion porukama ne ispisuj password.
Ako fake nema write counter, dodaj ga ili usporedi record/sequence i reboot;
sama usporedba getter vrijednosti ne dokazuje očuvanje novijeg formata.

**Provjere:** `cargo test --locked -p ruzina-settings` i target check.
**Gotovo:** sačuvani su i podaci i status/format; nema downgradea.
**Commit:** `fix(settings): preserve existing network records during migration`.

### A07 — ograniči neuspjele background MP3 index pokušaje

**Datoteke:** `firmware-rust/apps/m1-firmware/src/deck_audio_source.rs`
(`PreparedAudioSource`, oba mjesta konstrukcije, `scan_frame_index` i full
PCM queue grana); novi mali policy modul
`firmware-rust/crates/ruzina-audio-runtime/src/index_scan.rs` i export iz lib.rs.
Pročitaj `decode.rs::index_read/provide_index` te `mp3_index.rs`.

**Odluka:** skeniranje je opcionalno. Neuspjeli scan vraća false, čime postojeći
owner odradi 1 ms yield. Nakon tri uzastopna neuspjela pokušaja skeniranje se
isključuje za taj PreparedAudioSource. Novi LOAD resetira policy; običan seek
ga ne resetira. Uspješan scan prije limita resetira broj grešaka. Ovo je
početna bounded-retry politika, ne tvrdnja da 1 ms kvalificira board timing.

**Koraci:**

1. Dodaj no_std policy tip s `can_attempt()` i `record_result(accepted)`;
   brojač saturira na 3. Ne dodavati heap, wall clock ili broker u taj tip.
2. Polje policy inicijaliziraj u oba PreparedAudioSource konstruktora.
   Prije `index_read/read_at` provjeri can_attempt; disabled vraća false
   bez novog broker zahtjeva.
3. `read_at` sa zero bytes znači neuspjeh, ne napredak. `provide_index == false`
   također znači neuspjeh. Nemoj ignorirati njegov bool. True vrati samo kada
   su valjani bytes prihvaćeni. Više od traženog maksimuma tretiraj kao loš
   odgovor, ne kao dokaz uspješnog skeniranja.
4. Zadrži povrat file handlea i buffera kroz sve grane. Ne mijenjaj decoder
   generation, pending PCM block, loop checkpoints ili playback state zbog
   opcionalnog index kvara. Sačuvaj već valjani parcijalni indeks.
5. Nema lažnog `SourceDuration::Scanned` na odustajanju. Postojeća objava
   trajanja smije nastati samo kad indeks stvarno završi; najviše jednom.

**Testovi:** policy 2 failures→success reset; 3 failures→nema daljnjih pokušaja;
novi source opet može skenirati. Dodaj host harness koji koristi isti policy
i SourceDecoder/index put uz skriptirani čitač: 0-byte odgovori, rejected
range, kasniji uspjeh i odustajanje. Provjeri broj readova i očuvanje PCM/index
stanja. Čisti policy unit test nije dokaz da ga firmware owner koristi:
diff mora pokazati stvarni call site, a target compile mora obuhvatiti owner.
Fizičku latenciju i fairness brokera ostavi otvorenima za board test.

**Provjere:** `cargo test --locked -p ruzina-audio-runtime`, runtime-owner
testovi i `integration-check,render-graph-bringup` target check iz odjeljka 4.
**Gotovo:** ponovljeni failed readovi ne stvaraju neograničen broker loop;
neuspješni pokušaj prolazi kroz postojeći yield.
**Commit:** `fix(audio): bound retries for background MP3 index scans`.

### A08 — prikaži stvarni raw PITCH fader u Webu

**Datoteke:** `firmware-rust/crates/ruzina-web/src/json.rs`, `web/app.js`,
`web/tests/app.test.cjs`, `tests/http_server.rs`; simulator `src/web.rs`.
Izvor vrijednosti je `ruzina-deck/src/lib.rs::DeckState::pitch_raw`.

**Odluka:** JSON dobiva aditivno polje `pitch_raw` (u16, 0..16383).
`pitch_percent` ostaje postojeći prikaz tempa. Ne računaj raw vrijednost
inverzijom efektivnog pitcha: sync/range/nudge nisu položaj fizičkog fadera.

**Koraci:** dodaj polje DeckStatusu, EMPTY centar 8192 i JSON writeru; ažuriraj
sve konstruktore/fixtureove. Simulator čita `session.decks.deck(...).pitch_raw`.
I za IDLE deck prenesi njegov stvarni raw fader, umjesto ranog vraćanja EMPTY
koji bi ga uvijek nacrtao u centru. U `renderDeck` pozovi postojeći showValue
za `[data-continuous="pitch"]` i `data.pitch_raw`. Missing polje ostavi bez
promjene slidera. Ne emitiraj input/control pri programskom renderu.

**Testovi:** 0, 8192, 16383; prvi status s necentralnim faderom; vanjska
promjena; isti raw s drugim effective pitchom; IDLE status; aktivni drag ne
prepisuje slider, nakon završetka draga sljedeći status ga osvježava. Zadrži
postojeću kompatibilnost JSON polja i bounded writer overflow testove.

**Provjere:** ruzina-web i simulator testovi, Node suite, target check.
**Commit:** `fix(web): synchronize pitch sliders from raw deck state`.

## 4. A09 — završna verifikacija i dokaz završetka

U čistom build okruženju provedi nakon svih code izmjena:

```sh
# cwd: firmware-rust
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
node --test crates/ruzina-web/web/tests/app.test.cjs
cargo check --locked -p desktop-simulator
cargo test --locked -p ruzina-service --features runtime-owners --test runtime_owners -- --nocapture --test-threads=1
cargo test --locked -p ruzina-service --features runtime-owners --test composed_failures -- --nocapture --test-threads=1
cargo clippy --locked -p ruzina-service --features runtime-owners --tests -- -D warnings
cargo run --locked -p desktop-simulator -- --smoke target/ui-smoke
python3 tools/check_ui_goldens.py target/ui-smoke
python3 tools/check_docs.py
cargo check --locked -p ruzina-uac -p ruzina-web -p ruzina-ota -p ruzina-settings --target riscv32imafc-unknown-none-elf
```

Firmware ima vlastiti manifest/config. Sljedeće izvrši iz
`firmware-rust/apps/m1-firmware/`, ne iz workspace korijena:

```sh
cargo +1.95.0 check --locked --features integration-check,render-graph-bringup
cargo +1.95.0 check --locked --features board-jc1060
cargo +1.95.0 check --locked --features board-jc8012
```

Ovo su ciljane lokalne provjere, ne zamjena za cijeli P4 CI feature matrix.
Kad postoji autorizirani push/PR, oba postojeća workflowa moraju se provjeriti
na točnom pushed HEAD-u, uključujući board builds, package checks, APTA
exclusion i ostale postojeće korake. Ne kombinirati `integration-check` i
board selector kako bi se zaobišla namjerna zabrana.

Pregledani baseline imao je 911 passing testova. Novi rezultat normalno može
biti veći; sam broj nije kriterij kvalitete. Nula novih ignored/waived regresija.
Kod infrastrukturne greške zapiši NOT RUN/INFRA, ne PASS. Za dokumentacijski
plan bez code izmjena ove opsežne testove nije potrebno ponovno pokretati.

Uz rezultate zabilježi SHA, OS/toolchain, popis izmjena, nove testove, broj
passed/failed/ignored, UI screenshot pregled i preostale fizičke provjere.
Svaki R nalaz zatvara se dokazom, ne samo oznakom DONE.

## 5. A10 — uskladi dokumentaciju sa stvarnim stanjem

**Datoteke:** `README.md`, `docs/DEVELOPMENT.md`, `docs/HANDOFF.md`,
`firmware-rust/apps/desktop-simulator/README.md`,
`docs/M2_5_PARITY_PLAN.md` i relevantne postojeće status tablice.
Dodaj `docs/DEVELOPMENT_STATUS.md` kao jedno aktualno mjesto statusa; ostali
dokumenti neka sažimaju i linkaju, umjesto da održavaju različite brojke.

**Koraci:**

1. Inventariziraj sve RP stavke. Za svaku upiši: implemented shared logic,
   host test, simulator connected, target compiled, board adapter connected,
   hardware verified, next gate. Koristi PASS/PARTIAL/OPEN/NOT RUN/N/A s
   konkretnom datotekom ili testom. N/A mora imati razlog.
2. README uklanja zastarjele tvrdnje da PDB/S3CP/CUE/playlists još nisu
   implementirani. To ne znači da je njihov fizički media path potvrđen.
3. DEVELOPMENT i simulator README moraju priznati postojeći Web server/UI,
   ali jasno navesti rute koje još daju 501 i simulator RAM/demo granice.
4. HANDOFF ažurira current branch/base, dovršene fixove i sljedeći zadatak.
   Stari handoff i brojeve poput 315 tests premjesti pod datirani povijesni
   kontekst; nemoj ih prikazivati kao aktualan rezultat.
5. Jasno napiši: Guition start pokreće scheduler/dijagnostiku; product owneri
   još nisu puni board runtime. Crate check nije dokaz wiring integracije.
6. U source reviewu zadrži nalaze kao povijesni snapshot, dodaj kratku tablicu
   resolution SHA/test tek kad fix postoji. Ne prepisuj ga tako da izgleda
   kao da su popravci bili dio izvornog pregleda.

**Provjere:** link checker, `git diff --check`, ručna usporedba tvrdnji s kodom.
**Gotovo:** čitatelj iz README-a može doći do statusa i next taska; nigdje nema
tvrdnje da se samo čeka ploča dok postoje nepriključene host integracije.
**Commit:** `docs: align development status with verified runtime coverage`.

## 6. Faza B — posao koji može slijediti prije dolaska ploče

Faza A je konačan obvezni fix batch. Sljedeće integracije provode se nakon
njega, zasebnim malim promjenama. Ne dodavati ih u fix commitove. Pri početku
svake integracije prvo zapiši request/response fixture i acceptance test;
ako detalj ugovora nedostaje, zapiši otvorenu odluku prije API implementacije.
Ne obećavati da postojeći traits ili 501 route već čine gotovu funkciju.

### B01 — contract matrica preostalih ruta

Polazište: `ruzina-web/src/router.rs`, `server.rs`, `http.rs`, `parse.rs`,
`ruzina-settings/src/owner.rs`, `network.rs`,
`ruzina-controller-profile/src/registry.rs`, `semantic.rs`,
`ruzina-ota/src/install.rs`, `pull.rs`, `bundle.rs`, `boot.rs`.

Napravi `docs/WEB_INTEGRATION_CONTRACT.md`: za svaku dolje navedenu rutu
opiši method, bounded body, response schema, status kodove, owner poziv,
admission, persistence i cancel/timeout. Potvrdi API potpise iz aktualnog
koda, ne iz naziva zamišljenih `upload.rs` ili `owner.rs` datoteka.

| Skupina | Postojeće rute | Prvi dokaz |
|---|---|---|
| Settings | GET/POST `/api/ota/config` | Round trip kroz SettingsOwner i fake flash reboot |
| Profiles | GET `/api/controller-profiles`, POST `/api/controller-profile` | Validacija pa atomarna aktivacija registra |
| OTA | GET `/api/firmware`, POST `/api/ota/p4` | Signed install preko fake inactive slota i boot modela |
| Observability | GET `/api/resources`, GET `/api/diagnostic-log` | Bounded snapshot i redakcija osjetljivih podataka |

Recorder rute ostaju dokumentirani 501. Ne izmišljaj pull trigger rutu:
prvo uskladi postojeći OTA pull ugovor i način upravljanja. Do odluke je OPEN.

### B02 — body transport kao zaseban preduvjet

A04 obrađuje samo header-only rute. Prije novih POST body ruta dodaj bounded
streaming ugovor koji razdvaja admission headera od primanja bodyja i finisha.
Profile/config mogu imati strogo ograničen buffer prema postojećim parser
limitima; OTA image nikad ne stane cijeli u Vec. Head parser i guard ostaju
zajednički; chunked input ostaje odbijen dok zasebno nije projektiran.

Prvi testovi prije wiring-a: one-byte chunks, partial body + EOF, timeout,
declared length premalen/prevelik, backend Busy, cancel, drugi upload dok prvi
traje, chunk nakon cancela. Neaktivni zahtjev ne smije dalje pisati ni aktivirati
rezultat. Simulator adapter mora zadržati fairness i limite iz A04.

### B03 — settings i profile integracija

Prvo poveži settings rute preko stvarnog SettingsOwnera i testnog flash
adaptera. GET vraća javni/redigirani oblik; password se ne vraća u JSON,
queryju ili logu. POST validira cijeli payload prije upisa. Prazan ili pogrešan
payload ne briše postojeću konfiguraciju. HTTP uspjeh znači owner je prihvatio
dogovorenu operaciju; test razlikuje accepted i persisted ako su asinkroni.
Testiraj reboot, odbijanje novijeg recorda i prekid zapisa.

Zatim profile rute: koristi postojeće S3CP parsing/semantic provjere, staging
po uploadu i atomarnu aktivaciju tek nakon pune validacije. Loš version/CRC,
truncation, pogrešan control mapping, disconnect i storage failure ostavljaju
stari profil aktivnim. Test mora poslati HTTP bytes kroz server u registry i
dokazati ponašanje controller eventa nakon promjene, ne samo vratiti 200.

### B04 — signed OTA kroz host kompoziciju

Poveži upload put s postojećim bundle/install/boot modelom i fake flashom.
Prvo testiraj cijelu putanju bez TLS-a i bez board pisanja: valjan potpis/hash,
pogrešan ključ/hash, odbijeni downgrade prema postojećoj politici, prekid svakog
staging/commit koraka, cancel, concurrent Busy, reboot/rollback. Ne kreirati
novi format potpisa ili produkcijski ključ. OTA status ne smije tvrditi da je
nova slika boot-confirmed prije potvrde boot ownera. Pull test koristi fake
transport; TLS/certifikati i stvarni C6 network ostaju zasebni target gateovi.

### B05 — resources, diagnostics i fixture pokrivenost

Resources/log rute čitaju bounded owner snapshot bez blokiranja audio puta.
Definiraj units, missing vrijednosti i truncation/cursor ponašanje iz ugovora
B01. Simulator označava fake vrijednosti; ne izmišlja heap/temperature mjerenja.
Testiraj redakciju credentials, log overflow i sporog čitatelja.

Za Rekordbox/USB fixtureove napravi manifest provenance/dozvole prije dodavanja.
Pokrij nested playlists, više medija s istim lokalnim track ID-em, stale lease,
PDB title index 17 i S3CP v2/v3/v4 bez promjene v2 bytes. Stvarni controller
descriptor capture koristi tek kad je dostupan; synthetic fixture označi kao
synthetic. Dodaj bounded mutation/no-panic testove; puna fuzz kampanja mora
imati seed, trajanje, corpus i rezultat, a ne samo naziv alata.

**Faza B gotova:** relevantne rute više ne daju 501, a test prolazi kroz
HTTP→stvarni owner→fake adapter→reboot/rollback gdje je primjenjivo. To je
host integration PASS; fizička pohrana, USB i OTA na uređaju ostaju OPEN.

## 7. Faza C — redoslijed po dolasku ploče

Priprema prije dostave: sastavi evidencijski predložak po modelu i karticu
vlasništva periferija iz postojećih board dokumenata. Nepotvrđene veze ostaju
označene OPEN. Ne uključuj product feature u board start samo da nestane gate.

| Red | Rad | Uvjet prelaska dalje |
|---|---|---|
| C01 | Identitet modela, unit oznake, silicon readout; factory backup i ROM recovery | Backup checksum/readback i dokumentiran način oporavka |
| C02 | Minimalni boot, flash probe, PSRAM probe | Logovi i hash točne slike; 64 KiB CPU-cached test ne znači puna PSRAM/DMA kvalifikacija |
| C03 | Memorija/allocator, display/backlight/touch; zatim SD | Mjerenja i lifecycle za svaki model, potvrđeni pinovi/payload |
| C04 | USB PHY, port ownership, VBUS/backfeed; MIDI/LED, MSC/import | Stvarni attach/detach/reconnect i izolacija grešaka |
| C05 | Jedan deck audio pa dva; UAC, DAC MAIN switch/fallback | Slušni i telemetry dokaz; slušalice ostaju na kontroleru u svim prijelazima |
| C06 | Settings/Hot Cue persistence, C6/Web, signed OTA/rollback | Stvarni power-cut/cancel/timeout, bez gubitka valjanog starog stanja |
| C07 | Stress i soak na kandidatu | SHA/hash, trajanje, dropout/underrun/memory mjerenja i operator rezultat |

JC1060 touch reset/IRQ, JC8012 GSL3680 payload/provenance, stvarni panel i
USB-FS mapping/host power ostaju eksplicitna pitanja dok ih dokaz ne zatvori.
Rezultat s jedne ploče ne prenosi se automatski na drugu. Donor board rezultat
iz Pajoniiir M2.5 također ne kvalificira Guition runtime.

## 8. Dnevnik i predaja sljedećem modelu

Kopiraj ovaj predložak u radni zapis za svaki završen zadatak:

```text
Task: Axx / Rxx
Base SHA:
Status: OPEN | IN PROGRESS | VERIFIED | BLOCKED
Changed files:
Regression: naziv testa; potvrđen pad prije fixa; razlog pada
Implementation: kratko što se promijenilo
Validation: točna naredba; exit code; rezultat; lokacija loga
Commit SHA: ili uncommitted
Unrun gates: hardware / browser smoke / CI / ostalo
Next task:
```

Početna checklist lista (nijedna implementacija nije provedena ovim planom):

- [ ] A00 baza i izolacija
- [ ] A01 UAC
- [ ] A02 Web library snapshot i Node CI
- [ ] A03 selection
- [ ] A04 nonblocking WebPort
- [ ] A05 ordering
- [ ] A06 migration
- [ ] A07 index retry
- [ ] A08 pitch
- [ ] A09 završni gateovi
- [ ] A10 dokumentacija
- [ ] B01–B05 naknadna host integracija
- [ ] C01–C07 fizička integracija i kvalifikacija

**Početni prompt za implementacijski model:**

> Pročitaj ovaj plan i povezani code review. Provjeri radni checkout i remote
> prema A00, sačuvaj postojeće izmjene. Implementiraj fazu A točno redom,
> s regresijom prije svakog fixa. Nemoj raditi faze B/C u istom batchu.
> Koristi zadane politike za library snapshot, browser selection, Busy,
> network migration i MP3 retry. Nakon svakog zadatka ažuriraj dnevnik.
> Ne tvrdi hardware PASS dok ploče nisu fizički testirane. Na kraju izvrši
> A09 i A10 te predaj rezultate, preostale gateove i sljedeći zadatak.
> Ovaj plan sam po sebi ne autorizira push, merge, flash ili release.
