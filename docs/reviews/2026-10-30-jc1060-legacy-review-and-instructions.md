# JC1060 v1.3: pregled i upute za nastavak

Datum: 2026-10-30. Opseg: pregled postojećeg eksperimenta i plan za sljedećeg
implementacijskog agenta. Ovaj pregled nije izvršio nove pokuse na COM3 niti
izmijenio firmware. Zadnja korisnička potvrda: tvornički demo ponovno je vidljiv.

## 1. Odluka i radna lokacija

Nastavi s odvojenim ESP-IDF 6.1 platformskim slojem i postojećom Rust logikom.
Dosadašnji rezultat opravdava daljnju procjenu, ali još ne dokazuje izvedivost
cijelog proizvoda na v1.3. Ne započinji opći rewrite u C niti port esp-hal-a.

- Repo: `https://github.com/dvucinozd/Ruzina.git`.
- Aktivni worktree: `C:/Users/Daniel/.codex/worktrees/m1-guition-bsp/M1_PCB`.
- Grana: `ogranak/jc1060-c01-c02`; HEAD
  `4fe18e986e0a97d7f5eec0e68303f96481928af4`.
- Pregledane izmjene još nisu commitane. `git diff` sam ne prikazuje nove
  datoteke: obavezno pregledaj i `git status --short` i sadržaj `firmware-idf/`.
- Remote `main` provjeren s `git ls-remote`: isti SHA; continuation grana
  još ne postoji na remoteu. Primarni `D:/AI/M1_PCB` ostavi s njegovim
  nepovezanim untracked datotekama.
- Prvo pročitaj [arrival report](../boards/JC1060_ARRIVAL_2026-10-30.md),
  [unit record](../boards/readiness/jc1060-20261030-com3.json),
  [eksperiment](../../firmware-idf/legacy-jc1060/README.md) i ovaj plan.

ESP-IDF upravlja startupom, memorijskom/ROM mapom, FreeRTOS-om, interruptima,
driverima i alokacijom. Rust zadržava dijeljene algoritme i state machineove.
Za početak koristi `no_std` staticlib i uzak C ABI. Native esp-hal/esp-rtos
startup ne smije se povezati u isti executable. Native minimum 302 ostaje;
ovaj legacy profil je zaseban. JC8012 nema rezultate ovog fizičkog uređaja.

## 2. Što je provjereno i vrijedi zadržati

| Stavka | Dokaz i stvarni doseg |
|---|---|
| Identitet | Sačuvani esptool/eFuse dokazi: P4 v1.3, combined 103, MAC `e8:f6:0a:e7:22:1d`, flash 16 MiB. Nije danas ponovo očitavano s uređaja. |
| Backup | Danas ponovno hashirana oba neovisna puna dumpa i obje sekundarne kopije; sva četiri imaju 16,777,216 bajtova i isti očekivani SHA-256. |
| Build inputs | Svih osam source hashova u RAM package indexu odgovara trenutnim datotekama. Lokalni IDF HEAD odgovara pinu, bez tracked/untracked promjena u root statusu. |
| Pakiranje | Oba ZIP-a ponovno hashirana; podudaraju se s package indexom. Postojeći RAM BIN također odgovara zabilježenom hashu. |
| Rust integracija | C poziva stvarni `ruzina-deck::tempo_centipercent_from_raw`, bez kopiranja algoritma. Scalar ABI i panic->abort prikladni su za ovaj uski pokus. |
| Link | Sačuvani RAM map povezuje Rust simbol i matching `ilp32f` libgcc; ABI provjere nisu isključene. Ne izvodi zaključak o svim budućim compiler helperima iz jednog mapa. |
| Izvršavanje | Sačuvani logovi bilježe IDF 6.1, 360 MHz i četiri očekivana rezultata; dvije ranije RAM izvedbe. Današnji pregled nije nova fizička izvedba. |
| Oporavak | Nakon RAM pokusa USB reset nije vratio tvornički app; uslijedili su watchdog resetovi. Potpuni prekid napajanja i kasniji ROM/factory boot jesu opaženi, korisnik vidi demo. |
| PSRAM/display | 32 MiB PSRAM i vidljivi demo pripadaju tvorničkom firmwareu. Naš IDF backend još ih ne inicijalizira ni kvalificira. |

Backup SHA-256:
`27d5782978aeb3c42f8b49659e793d3ca8f59543f62e419d7b1981de2c492060`.
RAM BIN SHA-256:
`fc12b85de992b0f71ba7a6b552bc68df6562498e9256cc38a30a7828f8fcc9bb`.
ZIP SHA-256:
`ae25969832ca953eebca06bca45fae4669c95619d930dd829865459f45e4a061`.
To su identiteti postojećih dokaza; novi build nije automatski isti paket.

## 3. Nalazi koje treba riješiti prije proširenja pokusa

### R1 — P1: RAM loader ne provjerava odobreni image hash

Datoteka izvan repoa:
`D:/BoardEvidence/JC1060-20261030-COM3/run_ram_probe.py`.
`data` se čita i hash se poslije zapisuje, ali nema usporedbe firmware SHA-256
s odobrenim manifestom prije `load_ram`. Bilo koji drugi image s prihvatljivim
segmentima/revizijom na istom putu mogao bi biti izvršen. Provjere backupova,
segmenata i uređaja koriste `assert`, pa nestaju pri `python -O`.
`load_ram` ponovno otvara putanju; provjera ranije pročitanih bajtova sama ne
zatvara mogućnost promjene datoteke između provjere i učitavanja.

Ispravak je S2. Dokazi postojećeg izvršavanja i hashovi jesu sačuvani; nalaz
ne znači da je dokazano bio učitan pogrešan image. Tvrdnju arrival reporta
da je hash prije load-a bio provjeren treba suziti: ta zaštita nije kodirana
u pregledanom helperu; eventualna ručna provjera nije reproducibilni guard.

### R2 — P1: reset recovery nije završen, helper otežava analizu neuspjeha

Isti helper u `finally` uvijek radi `esp.hard_reset()`, uključujući neuspjeli
load/capture. Ako reset ili naknadno čitanje baci iznimku, moguće je preskočiti
`close()` i zapis result JSON-a. Provjere identiteta su prije tog `try`, pa
ni odbijanje uređaja nema zajamčeno zatvaranje porta. Exit code gleda scalar
match, ne ishod recoveryja. To nije PASS cijelog scenarija.

Nema dokaza da je hardverski kvar uzrok. U pinanom IDF-u
`components/esp_system/port/soc/esp32p4/clk.c:215` PURE_RAM put isključuje
MSPI flash clock. To je hipoteza za reset problem, ne potvrđen uzrok.
Nemoj nasumično pisati registre, ukloniti watchdog ili tvrditi da će flash
build sigurno riješiti reset. Primijeni S2 i S4.

### R3 — P2: ponovljiv build i CI ne obuhvaćaju novi backend

Root/main CMake provjerava target, legacy profil, 360 MHz, odsutnost PSRAM-a
i IDF verziju, ali ne veže Rust archive za trenutne izvore. Postojeća `.a`
datoteka dovoljna je da se prihvati, čak i ako je zastarjela. IDF version macro
ne dokazuje točan Git commit/submoduleove. Jednokratni build helperi nalaze
se samo u `D:/BoardEvidence/...`, s apsolutnim putanjama.

Oba postojeća workflowa izostavljaju `firmware-idf/**`; zasebni Rust workspace
nije obuhvaćen njihovim workspace fmt/clippy/test naredbama. Sam prolazak
native CI-a stoga ne provjerava ovu integraciju. `check_docs.py` također ne
obilazi README datoteke ispod `firmware-idf/`. Ispravak je S1/S3.

### R4 — P2: unit record miješa povijesni pokus i trenutno stanje

`recovery.normal_factory_boot` još kaže `DISPLAY_ACCEPTANCE_OPEN`, dok
`observed.factory_display_operator_acceptance` bilježi korisnikovu potvrdu.
U `legacy_idf_experiment` ostaje `physical_power_cycle=NOT_RUN` iz run-2,
uz kasnije `cold_recovery_after_ram=OBSERVED...`. Povijesni runtime JSON
ispravno opisuje tadašnji pokus i ne treba ga prepisati. Živi agregat mora
nedvosmisleno razlikovati kraj pokusa od kasnijeg oporavka. Ispravak je S0.

### R5 — granica dokaza: integer scalar test nije FPU/DSP niti port proizvoda

Trenutna funkcija i četiri scalar povratne vrijednosti ne provjeravaju float
argumente, SIMD/DSP, preotimanje zadatka, ownership buffera, allocator ili
audio rokove. Libgcc redoslijed zadrži uz map provjeru; ne generaliziraj ga
na sve archiveove. FPU/memoriju provjerava S5, a produktne drivere S6.

## 4. Izvedbeni plan — jedan korak, provjera, pa sljedeći

S0–S3 su rad bez uređaja. Odradi ih redom bez otvaranja COM3 i bez čekanja
fotografija. Ako fizička radnja treba korisnika, pripremi konkretan postupak
i nastavi neovisni build/test rad. Ovaj plan nije naredba da sada flashaš.

### S0 — uskladi autoritet i sačuvaj početno stanje

1. Zabilježi `git status --short`, HEAD, remote SHA i popis novih datoteka.
   Ne resetiraj worktree. Ne regeneriraj unit JSON starim vanjskim
   `record_unit.py`: time bi nestale kasnije ručno zapisane opservacije.
2. U unit recordu odvoji nepromjenjivi `probe_run` s linkom na postojeći
   runtime JSON od kasnijeg recovery događaja. Događaji trebaju scenario,
   izvor dokaza i status; ne izmišljaj vrijeme koje nije zabilježeno.
3. Ažuriraj zastarjeli display status: factory demo owner-confirmed,
   naš display NOT_RUN. Cold transition log ostaje NOT_CAPTURED,
   factory restore NOT_RUN, RAM warm reset FAIL.
4. Uskladi arrival report i HANDOFF. Dodaj mali validator unit-recorda
   u budući `firmware-idf/legacy-jc1060/tools/`, uz test kontradiktornih
   statusa i nepostojećih relativnih evidence linkova. Povijesni neuspjeh
   i kasniji uspješan recovery moraju moći koegzistirati.

Gotovo kada validator prihvati aktualni zapis, odbije kontradiktorni fixture,
a sačuvani runtime logovi, JSON i izvorni ZIP ostanu byte-identični.

### S1 — uvedi ponovljiv build bez uređaja

Predložene nove datoteke: `tools/build_legacy.ps1`, `tools/package_legacy.py`
unutar `firmware-idf/legacy-jc1060/`; host testovi u `tools/tests/`.

1. Parametri: repo root, clean IDF root, Python/tool paths, varijanta
   `flash-build-only` ili `pure-ram`, novi output dir. Izvedi putanje iz
   parametara, ne ugrađuj Danielove putanje. Odbij miješanje build/config
   direktorija između varijanti i prepisivanje zamrznutog evidence paketa.
2. Provjeri IDF SHA `fff9895c82d744c7237be8847347bdd1b07c6643`, submodule
   SHA/status i izmjene; GCC `esp-15.2.0_20251204`, Rust 1.95.0, target
   `riscv32imafc-unknown-none-elf`, esptool 5.6.0. Verzije CMake/Ninja/Python
   zabilježi i zadrži početni poznati build okoliš (4.0.3/1.12.1/Python3.13).
3. U istoj naredbi prvo izgradi Rust `--locked --release`, pa pokreni IDF
   s točno tim archiveom i zasebnim apsolutnim SDKCONFIG/defaults putanjama.
   Ne oslanjaj se na postojanje prethodne `.a`; build failure prekida lanac.
4. Manifest uključuje hash archivea, ELF/BIN/map/configa, source commit plus
   dirty status/hash promjena, dijeljene Rust dependency izvore, toolchain
   verzije, image header/segmente i varijantu. Osam lokalnih source hashova
   nije puna provenance dijeljenih crateova. Pinani lockfileovi ostaju.
5. Image/map provjere: P4 legacy 100..199, očekivani ABI, stvarna Rust
   referenca, bez native startup vlasnika, bez `--no-warn-mismatch` ili
   sličnog isključivanja provjera. Broj/položaj segmenata uzmi iz novog
   imagea; stare granice služe samo za odobreni stari RAM paket.
6. Build i package alat nemaju funkciju flash/load/reset. Ne prikazuj
   generički IDF write-flash ispis kao odobren programming plan.

Gotovo: čisti build obiju varijanti u novim direktorijima, manifest veže
stvarno povezane izvore i archive. Stari BIN hash nije obvezan za novi build;
svaka razlika mora imati novi paket i ne nasljeđuje fizički PASS.

### S2 — zamijeni jednokratni loader provjerljivim alatom

Predložene datoteke: `tools/run_ram_probe.py`, `tools/tests/test_ram_probe.py`.
Preuzmi potrebnu logiku iz vanjskog helpera, ali originalni dokaz sačuvaj.

1. Default je offline `verify-only`; manifest, image i output su eksplicitni.
   Provjeri očekivani hash i duljinu prije otvaranja porta. Koristi `if/raise`,
   nikada `assert` za zaštite. Sve zaštite moraju raditi i s `python -O`.
2. Provjeri chip/header/revision, entrypoint unutar izvršivog segmenta,
   nenulte duljine, overflow/overlap i dopuštene SRAM/SPM raspone izvedene
   iz pinanog legacy izvora. Odbij flash-mapped segmente i pogrešnu varijantu.
3. Spriječi zamjenu validiranog imagea prije učitavanja: koristi podržani
   in-memory input ako esptool API to dopušta ili zaključanu privatnu kopiju
   čiji sadržaj i hash ostaju isti tijekom load-a. Ne pretpostavljaj API.
4. Tek u eksplicitnom execute modu otvori uređaj; prije load-a provjeri
   rev103/MAC i backup manifest s dostupnim punim kopijama. Port je parametar;
   COM broj nije identitet. Nijedan helper ne smije pisati flash/eFuse.
5. Izoliraj transport iza malog sučelja za mock. Zatvaranje porta osiguraj
   vanjskim `finally`, uključujući identitet/timeout/load/reset iznimke.
   Zapiši djelomični log i terminalni rezultat čak i kad reset/capture zakaže.
6. Ne radi automatski hard reset u generičkom cleanupu. Scenarij mora
   unaprijed definirati reset policy i operator recovery. Zasebno evidentiraj
   scalar rezultat i recovery rezultat; ukupni exit ne smije predstavljati
   neuspjeli recovery kao uspješan kompletni scenarij.
7. Testovi bez serijskog uređaja: promijenjen image/manifest, `python -O`,
   pogrešan MAC/revizija, segment overlap/out-of-range, postojeći output,
   timeout, load failure, reset failure. Odbijeni input mora imati nula load
   poziva; svaki otvoreni fake port zatvoren; originalni logovi neoštećeni.

Gotovo: svi negativni testovi prolaze, offline verify prihvaća zamrznuti RAM
paket, a nikakva nova fizička izvedba nije potrebna za prihvat ovog koraka.

### S3 — CI i provjere prije bilo kakvog novog fizičkog pokusa

1. Dodaj `.github/workflows/legacy-idf.yml` s putanjama `firmware-idf/**`,
   njegovim workflowom i dijeljenim Rust crateovima/lockfileovima koje koristi.
   Pokreći format, target Clippy `-D warnings`, alatne testove, oba builda i
   manifest/map provjere. Pinaj IDF commit; CI nikad ne pristupa uređaju.
2. Dodaj negativne configure testove: v3 profil, pogrešan target, CPU400,
   PSRAM u početnom scalar profilu i pogrešan IDF. Ne mora svaki slučaj
   ponovno kompajlirati cijeli SDK; mora pasti na namjeravanom guardu.
3. Proširi `firmware-rust/tools/check_docs.py` na IDF README datoteke,
   izuzimajući build output. Sačuvaj postojeće native workflowove.
4. Provjeri da izmjena samo IDF C/Rust izvora pokreće ovaj CI i da build
   ne potajno koristi lokalni `D:/BoardEvidence` ili stale Rust archive.

Lokalne polazne naredbe iz repo roota (ne komuniciraju s pločom):

```text
python firmware-rust/tools/check_docs.py
cargo +1.95.0 fmt --manifest-path firmware-idf/legacy-jc1060/rust-core/Cargo.toml -- --check
cargo +1.95.0 clippy --locked --release --target riscv32imafc-unknown-none-elf --manifest-path firmware-idf/legacy-jc1060/rust-core/Cargo.toml --target-dir firmware-rust/target/legacy-jc1060/rust -- -D warnings
git diff --check
```

Postojeći okoliš izvršava Rust iz WSL-a; Windows IDF zatim prima Windows
putanju do dobivenog archivea. Dokumentiraj to, ili osiguraj jedan čisti
CI/Linux put. Gornje naredbe nisu zamjena za testiranje novog build wrappera.

### S4 — dokumentiraj recovery i prvi flash-backed pokus

Prije uređaja pripremi `docs/boards/JC1060_LEGACY_RECOVERY.md` i novi vanjski
evidence direktorij. Nikad ne koristi postojeći `ram-run-2` za novu izvedbu.

1. Offline analiziraj factory backup: validiraj stvarnu partition tablicu i
   aplikacijske/boot headere; ne pogađaj offsete prema generičkom IDF primjeru.
   Napravi tablicu svake planirane promjene flash raspona, sadržaja koji se
   gubi i točnog povratka iz verificiranog dumpa. Ne ispisuj NVS tajne.
2. S korisnikom zabilježi napajanje, fizički konektor/oznake i ručnu ROM entry
   sekvencu. Manual ROM handshake treba raditi neovisno o aplikaciji.
   Factory restore i dalje označi NOT_RUN dok nije stvarno proveden.
3. Pregledaj reset put u pinanom IDF-u i semantiku USB reset mehanizma.
   Odvoji PURE_RAM specifičan problem od normalnog flash boota. Ne troši
   neograničen broj pokušaja na RAM reset prije normalnog IDF boota.
4. Ako se radi usporedni RAM test, mijenjaj jednu hipotezu, novi image/hash,
   operater dostupan za potpuni prekid napajanja. Prvi watchdog loop ili
   neuspjeli ROM flash attach prekida automatizirane pokušaje.
5. Kad C01 identitet/recovery/layout uvjeti budu zatvoreni, pripremi točan
   flash-backed scalar paket i programming plan. Ne flashaj iz ovog dokumenta
   bez te pripreme i važećeg korisničkog opsega rada. Postojeći factory
   bootloader ne kombiniraj s IDF6.1 aplikacijom na pretpostavku.
6. Prvi write prati readback/hash svih zapisanih raspona, zatim scalar pozivi,
   monotoni heartbeat i reset cause. Mjeri odvojeno power-on, aplikacijski
   restart, service USB reset i reconnect. Za USB reconnect zabilježi gubi
   li se i napajanje; takav događaj nije warm reconnect.
7. Početni smoke kriterij: tri uzastopna ponavljanja svakog primjenjivog
   scenarija bez watchdog petlje i izgubljenog recovery puta. To je novi
   predloženi kriterij, ne već ostvaren rezultat ili dugotrajni soak.

Gotovo: točan flash-backed image ima boot/recovery dokaze ili jasan FAIL.
PURE_RAM warm reset može ostati izolirano nepodržan ako se taj način ukine
iz redovnog bring-upa, cold recovery dokumentira i flash-backed reset put
neovisno prođe. Nemoj preimenovati stari RAM FAIL u PASS. Ako i normalni
IDF boot pokazuje isti problem, zaustavi širenje na periferije.

### S5 — memorija i stvarni C/Rust task/FPU dokaz

Ne uklanjaj PSRAM zabranu iz scalar profila bez zamjene: dodaj odvojeni
`memory-task` profil, a scalar baseline zadrži za dijagnostiku.

1. IDF sam inicijalizira PSRAM koristeći provjerene postavke za v1.3/modul.
   Izmjeri kapacitet; tvornički 32 MiB/200 MHz nisu automatski naše postavke.
   Počni ograničenim exclusive scratch blokom iz capability allocatora.
2. Pattern/address test, guard canary, allocation failure i heap-integrity
   provjere; evidentiraj internal/external free/minimum heap i task stackove.
   Nema pisanja po proizvoljnim fizičkim adresama ili tuđim framebufferima.
3. Dodaj zasebnu C/Rust float funkciju i poznate vektore s definiranim
   tolerancijama. Dva FreeRTOS zadatka na ISTOJ jezgri koriste različita
   deterministička stanja i stvarno se preotimaju tijekom float računanja.
   Potvrdi disassemblyjem da računanje nije optimizirano u konstante i da
   test stvarno izlaže živo FP stanje preotimanju; samo yield nakon gotovog
   scalar poziva nije FPU context test. Potom neovisno provjeri dvije jezgre.
4. Buffer ABI tek nakon scalar/float testa: eksplicitni pointer+length,
   status kod, jedna strana alocira i oslobađa; dokumentiraj trajanje posudbe,
   alignment i invalid input. Bez Rust Vec/String/reference layouta u C ABI-ju,
   callbackova iz ISR-a u neprovjeren Rust ili panica kroz C granicu.
5. Benchmark reprezentativnog postojećeg DSP bloka na 360 MHz: block/sample
   format, worst observed latency, preemption, memory footprint. Izvedi rok
   iz stvarnog audio block trajanja; ne izmišljaj PASS prag nakon mjerenja.
   CPU scratch test ne dokazuje DMA/cache coherence.

Gotovo: zasebni ABI/FPU/memory izvještaji s image hashom, brojem iteracija,
trajanjem, mismatchima i peak memorijom. Neuspjeh ima reproducibilan test.

### S6 — tek tada BSP i produktni adapteri

| Redoslijed | Posao | Uvjet prihvata |
|---|---|---|
| 1 | New_Panel JD9165, backlight, touch | Pinani izvori/licence/init table, potvrđeni power/reset pinovi; test boja/rotacije/uglova i ponovnog boota. Stari EK79007 log nije identitet drivera. |
| 2 | SD read-only | Točan power/pin owner, mount/read/uklanjanje, cancellation/generation fencing; tek potom persistence. |
| 3 | USB host | Potvrđena PHY/VBUS/backfeed topologija v1.3, zatim enumeration/disconnect i zasebni MIDI/MSC/UAC adapteri. Ne kopirati v3.2 register map. |
| 4 | Jedan pa dva decka | Isti Rust DSP/state machineovi, izmjerena memorija/rokovi, fizičko slušanje i underrun/reconnect stres. Slušalice uvijek kontroler; vanjski DAC samo stereo MAIN uz fallback. |
| 5 | C6/settings/Web/OTA | Vlasništvo flasha/mreže, povjerenje/slotovi/rollback i zasebni testovi. OTA nije prečac za prvi bring-up. |

Vendor izvor je `C:/Users/Daniel/Desktop/ESP-BOARDS/JC1060P470C_I_W_Y/1-Demo/Demo_IDF/ESP-IDF_5.5.4`.
Koristi [source manifest](../boards/readiness/jc1060-legacy-panel-sources-20261030.json).
Obje grane pozivaju JD9165, ali init tableovi su različiti. `New_Panel` je
korisnikova informacija, ne zamjena za provjeru stvarnog panela i touch žica.
Ne preuzimaj cijeli vendor demo ili wildcard dependencies u proizvod.
APTA ostaje OFF; ne mijenjaj native podršku ili JC8012 na temelju ovih testova.

## 5. Uputa agentu koju korisnik može izravno predati

> Pročitaj ovaj dokument i navedene izvore u aktivnom worktreeu. Provedi S0,
> S1, S2 i S3 redom, bez pristupa COM3. Za svaki korak ostavi promjene, stvarne
> rezultate testova i jasne preostale stavke; planirani test nikada nije PASS.
> Zatim pripremi S4 offline layout/recovery analizu. Ne preskači na ekran,
> PSRAM ili audio dok nisu zadovoljeni navedeni prethodni uvjeti. Sačuvaj
> postojeći RAM paket i factory backupove. Ako jedan korak zapne na fizičkom
> opažanju, dovrši neovisni host rad i navedi točno koju radnju treba korisnik.
> Ne snižavaj native minimum 302. Ne smatraj ovaj dokument odobrenjem za merge
> ili objavu releasea. Commit/push radi prema važećoj korisničkoj uputi,
> selektivno, bez binarnih dumpova i privatnih logova; nakon pusha provjeri SHA.

Za svaki dovršeni korak ažuriraj HANDOFF i execution log: promijenjene datoteke,
naredbe/exit code, paket/source hash, što je host/build/runtime/operator dokaz,
preostali blocker i sljedeća točna radnja. Ne ponavljaj cijelih 938 testova za
isključivo dokumentacijski popravak; promjena dijeljenih Rust crateova traži
njihove regresije i postojeće native/host gateove. Novi IDF backend uvijek
traži vlastite provjere.

## 6. Izvori i granice ovog pregleda

Pregledani su svi novi C/Rust/CMake/defaults izvori, unit/runtime/package JSON,
vanjski build/load helperi, postojeći CI i relevantni pinani IDF clock izvor.
Hash provjere su ponovljene; firmware build, puni host suite i fizički pokusi
nisu ponavljani u ovom reviewu. Dokumentacijski checker: 129 linkova PASS;
`git diff --check`: PASS (samo Git LF/CRLF obavijesti). Nedostaci su planirani za popravak, nisu
tiho implementirani tijekom pregleda.

- [ESP-IDF v6.1 release](https://github.com/espressif/esp-idf/releases/tag/v6.1):
  potvrđen pin `fff9895`; zadrži tu bazu pri reprodukciji.
- [esp-hal P4 support boundary](https://github.com/esp-rs/esp-hal/releases/tag/esp-hal-v1.2.0-rc.0):
  početna podrška za P4 v3.0 i novije ne omogućuje naš v1.3 native target.
- Lokalni pinani IDF `clk.c` s PURE_RAM MSPI granom služi za hipotezu R2;
  uzročnost traži zaseban pokus, ne citiranje tog retka kao gotovog rješenja.
