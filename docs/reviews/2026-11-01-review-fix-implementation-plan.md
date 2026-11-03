# JC1060: detaljne upute za implementaciju popravaka nakon reviewa

Datum: 2026-11-01. Status: OFFLINE_A_H_COMPLETE / BUILD_ONLY / NOT_FLASHED.

Napredak i dokazi: [execution log](2026-11-01-review-fix-execution.md).
Koraci A–H dovršeni su lokalno: svih 17 IDF paketa, host/target testovi i
završni CodeRabbit pregled prolaze. Fizički korak I pripremljen je u
[zasebnoj kampanji](2026-11-01-review-fix-device-campaign.md), ali nije pokrenut.

Ovaj dokument provodi nalaze R1–R7 iz [reviewa](2026-11-01-code-review-and-next-steps.md). Namijenjen je agentu koji mora raditi male, provjerljive promjene. Ne označavati korak dovršenim samo zato što se kod kompilira.

## 0. Obvezne granice i priprema

1. Radi u `D:/AI/M1_PCB`. Pročitaj `docs/HANDOFF.md`, review, ovaj plan, oba izvještaja `2026-11-01-legacy-deck-mix.md` i `2026-11-01-legacy-deck-engine.md`, te postojeći `ruzina-deck/src/load_admission.rs`.
2. Očitaj `git status --short --branch`, `git log -5 --oneline`, `git worktree list`; napravi `git fetch origin`. Zabilježi HEAD, origin/main i odnos grana. Pregledani HEAD bio je `b35dcd39f4c1ddb9f4b253d6af89110ee68a2335`; main `bd52402160af27a3e234f7647d703a847644817b`. Ako su se promijenili, usporedi relevantne datoteke prije primjene plana.
3. Postoje lokalne necommitane izmjene plana vanjskog audija, handoffa i reviewa. Sačuvaj ih. Ne radi reset, clean, automatski stash ni široko stageanje. Ako otvaraš novu granu, koristi prefiks `ogranak/` i polazište koje uključuje oba deck PR-a.
4. Ovo su offline popravci. Ne pristupaj COM3, ne flashaj i ne pokreći RAM probe. Native minimum ostaje 302; ova ploča je v1.3/103 i koristi zasebni IDF backend. Ne mijenjaj APTA status, DSP razine, DAC plan ni pinove.
5. Ne dodaj scratch/FX/OTA/UI funkcije u ovaj paket. Ne nadograđuj IDF, Rust ili dependency verzije radi ovih popravaka.
6. Napravi `docs/reviews/2026-11-01-review-fix-execution.md`. Za svaki korak zapisuj promijenjene datoteke, test prije/poslije, komandu, rezultat, otvorene nedoumice i stvarni status BUILD_ONLY/NOT_FLASHED.
7. Testove iz `evidence/2026-11-01-code-review/review-regressions.patch` prvo reproduciraj u izoliranoj kopiji. Provjeri patch s `git apply --check`. Ne koristi obrisanu prethodnu `/tmp` kopiju kao izvor. Četiri testa trebaju otkriti tri klase problema; konkurentni test ovisi o raspoređivanju niti.

## Zajednički ugovor prije uređivanja

Uvedi ili prilagodi tipove s jasnim identitetom, ne nevezane atomike za svaki podatak:

- identitet izvora: media lease/generation + deck + load generation;
- zahtjev učitavanja: identitet, catalog publication/index i stvarni `LoadPermit`;
- seek zahtjev: identitet izvora, request ID, cilj u ms;
- seek odgovor: isti identitet i request ID, rezultat `Ready / Eof / Failed / Cancelled`, stvarni položaj i rezultatna ring epoha;
- opis ringa: epoha, start, rate, origin i terminalno stanje pripadaju jednoj publikaciji.

Nazivi tipova su prijedlog. Prvo provjeri postojeće tipove i ponovno ih upotrijebi. Ne izmišljaj ID iz adrese objekta. Koristi equality za identitet generacija; nemoj uspoređivati wraparound brojače običnim `>` bez definiranog ugovora. Dokumentiraj kada se generacija mijenja. Nijedan odgovor ne smije promijeniti drugi deck ili noviji izvor.

USB client task jedini mijenja Engine/Mix i puni UAC. SD producer jedini mijenja dekoder i objavljuje PCM. C SD owner jedini upravlja SD/DMA. Status UI prima kopiju, ne mutabilni Engine pointer. Zadrži te granice; nemoj dodati globalni mutex koji drži audio callback dok traje čitanje kartice.

Redoslijed: A -> B -> C -> D -> E -> F -> G -> H. Sljedeći korak počni tek nakon njegovih lokalnih testova. R3 se popravlja prije seeka jer seek ovisi o pouzdanoj publikaciji izvora.

## A. Koherentna publikacija ringa — R3

Datoteke: `firmware-idf/legacy-jc1060/rust-core/src/deck_play.rs`, njegovi testovi; po potrebi `deck_mix.rs`.

1. Razdvoji PCM head/tail protokol od opisa izvora. Zadrži single-producer/single-consumer vlasništvo slotova.
2. Napravi snapshot API koji vraća cijeli opis ili `Busy`. Writer mora označiti početak promjene prije prvog polja, a završetak nakon zadnjeg. Sva polja koja se konkurentno čitaju moraju biti atomici ili zaštićena stvarno ispravnim mehanizmom; seqlock nad običnim Rust poljima nije opravdanje za data race.
3. Za prvu implementaciju radije odaberi konzervativne memorijske ordere i dokumentiraj dokaz konzistentnosti. Nemoj pretpostaviti da Release na posljednjem epoch inkrementu štiti od sljedećeg writera. Slabije ordere uvodi samo uz provjeru interleavinga.
4. Reader najprije čita i validira privremeni snapshot. Tek potom mijenja lokalni epoch/tail/rate/origin i objavljuje tail. Na `Busy` ne smije djelomično primijeniti novu poziciju. Bounded pokušaj; bez beskonačnog spina u audio zadatku.
5. Uskladi EOF s generacijom. Završetak starog izvora ne smije završiti novi.
6. Testiraj determinističke prekide prije/pod/poslije publikacije preko test-only hooka ili izdvojenog protokola, zatim threaded stress. Dodaj različite rate/origin vrijednosti, puni ring, ponovljene restarte i wrap brojača. Provjeri tail, ne samo epoch.

Gotovo: nemoguće je prihvatiti miješani opis; postojeći bit-exact decode/render testovi prolaze. Ne tvrdi da sam stress test dokazuje memorijsku ispravnost.

## B. Transakcija LOAD/PLAY — R1

Datoteke: `deck_engine.rs`, `controller_midi.rs`, `deck_play.rs`, `idf_media_bridge.rs`, `main/load_display.c`; referenca `firmware-rust/crates/ruzina-deck/src/load_admission.rs` i native owneri.

1. Pronađi sve ulaze LOAD-a. Trenutačni put je MIDI -> UI bridge -> `ruzina_legacy_play_load` -> producer. Admission i rezervacija moraju činiti isti kontrolirani postupak; UI ne smije drugi put stvarati nezaštićen zahtjev.
2. Dodaj po jedan `DeckLoadGate`. Na prihvaćenom LOAD-u rezerviraj permit i objavi koherentan zahtjev. Drugi LOAD dok je prvi aktivan odbij kao `LoadInFlight`; ne prepisuj slot i ne izgubi permit.
3. Na PLAY/jog transport claim upotrijebi `claim_for_transport`: rezerviran LOAD može biti otkazan; aktivni replacement odbija claim. Pri odbijanju pravilno razriješi postojeći playback request ID.
4. Producer poziva `begin_replacement(permit)` neposredno prije `close` starog filea i `ring.restart`. Ako je permit superseded, stari izvor, ring i cueovi ostaju isti.
5. Terminalni load odgovor nosi permit/generaciju. Engine prihvaća samo aktualni odgovor. Razlikuj otkazanu rezervaciju (stari izvor sačuvan) od neuspjeha nakon stvarne zamjene (stari file već zatvoren, novi deck nije spreman).
6. Drži zaštitu replacementa do obrade terminalnog odgovora; nemoj dopustiti PLAY u razmaku između producer ready i engine commit. Oslobodi gate na uspjehu, grešci, cancellationu i media-loss putu.
7. UI može prikazati `loading`, ali `deck_track` kao potvrđeni identitet postavi tek nakon uspješnog commita. Odbijen LOAD ne mijenja potvrđeni naslov.
8. Ne popravljaj problem samo uklanjanjem `unload_track`: producer je već mogao uništiti stari izvor.

Test matrica za oba decka: LOAD->PLAY prije replacementa; PLAY tijekom replacementa; A->B zahtjevi; kasni A ready/fail; neuspjeli open/decode; odspajanje izvora; odbijen LOAD ne mijenja UI/cueove; nakon svakog završetka novi LOAD opet može rezervirati gate. Provjeri stvarni producer/file close/ring, ne samo engine booleane.

## C. Završivi i identificirani seekovi — R2

Datoteke: `deck_engine.rs`, `deck_play.rs`, `idf_media_bridge.rs`, pripadni testovi.

1. Pending seek stanje postavi PRIJE objave zahtjeva produceru. Dodaj request ID i load/media identitet. Izbaci zaključivanje završetka samo preko `epoch != old_epoch`.
2. Producer prije izvršavanja provjerava identitet. Noviji seek može zamijeniti čekajući stari; već započeti stari rezultat ne smije razriješiti noviji zahtjev.
3. Svaki izvršeni seek završi odgovorom, uključujući `change` failure, read failure, decode failure, cancellation i EOF. U bridgeu odgovor dostavi Engineu; log je dodatak, ne jedini potrošač.
4. Razlikuj prazan ispravan EOF blok od faulted bloka. Za poznato trajanje cilj na/iza kraja clampaj na stvarni kraj, objavi terminalni epoch i stvarni položaj. Nepoznato trajanje rješava dekoder, bez izmišljanja trajanja iz tagova.
5. Na Ready engine prihvaća samo odgovarajući snapshot/epoch, resetira resampler i nastavlja prethodno traženo stanje. Na EOF zaustavi deck. Na Failed očisti hold, zaustavi/faultaj deck i ukloni stare buffered izlaze; ne nastavljaj prikazivati lažni položaj. Gubitak medija ide kroz D.
6. Razriješi držanje pozicije i kad nema UAC streama: control-owner tick treba konzumirati receiptove; receipt processing ne smije ovisiti isključivo o render callbacku.
7. Izričito testiraj epoch koji stigne prije sljedećeg engine ticka. Ne koristi `ended && empty` kao zamjenu za receipt jer vrijedi i prije novog seeka s EOF-a.

Testovi: WAV frame-exact naprijed/natrag, točan EOF i preko EOF-a, seek s EOF-a na nulu, MP3/FLAC, greška čitanja, dva seeks tijekom rada producera, kasni odgovor starog tracka, seek->LOAD, CUE hold/release, prekid kontrolera. Konačno stanje mora uskladiti položaj, PCM, Play LED, pending state i load admission.

## D. Revokacija medija i kontrolera — R4

Datoteke: `idf_media_bridge.rs`, `main/sd_bridge.c`, `deck_play.rs`, `deck_mix.rs`, `deck_engine.rs`, `controller_midi.rs`.

1. Napravi jedno idempotentno obavještavanje o gubitku media generationa. Koristi postojeću live lease provjeru, nemoj uvoditi drugi neovisni izvor istine.
2. C owner objavljuje revokaciju bez izravnog pristupa Engine/Mix objektima. Rust control/audio owner je konzumira čak i kad producer čeka I/O ili je izašao.
3. Za pogođene deckove poništi pending load/seek, gateove, loaded/playing, cue-preview i status. Odbij sve kasne odgovore te generacije.
4. Potrošač mora prestati uzimati stari PCM, poništiti cached `RenderOutputBlock` i resampler. Samo resetiranje ringa ne uklanja prethodno renderiran blok.
5. Pakete već predane USB driveru tretiraj prema izričito dokumentiranom drain/cancel ugovoru; ne tvrdi da se već poslani zvuk može povući. Mjeri gornju granicu do tišine.
6. Jedinstveni cleanup pozovi sa svih izlaza `serve_decks`, uključujući stale detail i allocation failure. Ne krši postojeći DMA-drain ugovor niti oslobađaj aktivne buffere.
7. Obradi `SessionDelivery::Connection` u MIDI drainu i proslijedi `set_controller_connected` efekte. Sačuvaj postojeću politiku običnog playbacka pri odspajanju; obvezno otpusti controller-owned holdove, bez izmišljanja automatskog resumea.
8. Zadrži trenutačni `reinsert=APP_RESTART_REQUIRED`. Automatski remount je poseban kasniji zadatak.

Testovi: kartica izgubljena uz puni ring, paused deck, seek, detail i LOAD drugog decka; oba decka na istoj lease; neovisna lease ostaje aktivna; revokacija usred cached bloka; kasni Ready i Seeked; disconnect s držanim CUE/jogom. Bez uređaja simuliraj signal i potvrdi tišinu i završeno stanje.

## E. USB audio recovery — R5

Datoteka: `main/usb_host_enum.c`; izdvoji mali C modul stanja ako je potreban za host test.

1. Nacrtaj postojeće startup/EP0/stream/stop putove. Evidentiraj vlasnika i životni vijek svakog transfera.
2. Uvedi eksplicitne slotove: idle, in-flight, retry-pending, draining; stream: off, starting, streaming, recovering, stopping, fault-held.
3. `busy` postavi samo nakon uspješnog submita. Na callbacku obradi transfer i packet statuse; odvoji submit, fill i completion greške u brojačima.
4. Retry zakazuj iz client loopa, ne rekurzivno u callbacku. Predloženi početni budžet: najviše tri uzastopna pokušaja po slotu, najmanje jedan client-loop prolaz između pokušaja; nakon toga eksplicitni fault i drain. Brojeve konfiguriraj kao imenovane konstante i označi ih kao još nemjerene na targetu.
5. Recovery mora biti kompatibilan sa stvarnim IDF API-jem za endpoint halt/flush/clear. Pročitaj header i postojeći primjer za pinned IDF prije uporabe; ne izmišljaj funkcije.
6. Parcijalni startup nije `STREAMING` uspjeh: dovrši potreban pool ili uredno zaustavi. Ne gubi pointer pri drugoj alokaciji.
7. Svaki callback/context veži uz connection generation. Ne recikliraj stari context dok postoji mogućnost callbacka. Pri timeoutu drainanja zadrži resurse i blokiraj novi session; ne oslobodi pa nastavi.
8. `gone` ima prednost nad retryjem i EP0 nastavkom. Bez automatske predaje starog PCM-a novom kontroleru.

Host fault injection na stvarnom izdvojenom state-machine kodu: submit fail, completion fail, fill fail, partial alloc, NO_DEVICE, unplug tijekom EP0, kasni callback nakon nove sesije, drain timeout. Provjeri točno broj ownership transitiona/free operacija, zabranu submita mrtvom uređaju i konačno stanje. Nemoj testirati kopiju logike napisanu samo za test.

## F. Ograniči rad producera — R6

Datoteke: `deck_play.rs`, `idf_media_bridge.rs`; eventualno reusable incremental API u `ruzina-media-load` uz očuvanje postojećih korisnika.

1. Zamijeni monolitni prime s `begin_load/begin_seek` + ograničenim `poll` koracima. Stanja: opening, priming, ready, ended, failed/cancelled. Zadrži pending decoded blok ako je ring pun.
2. Ne pretvaraj dosegnuti action/time budget u decode failure; to znači `Pending` i nastavak idući krug. Ne resetiraj dekoder pri svakom pollu.
3. Posluži playing ringove s najmanjim vremenskim zalihama prije detalja i novih loadova. Izračunaj zalihu iz frames/rate/pitch; ne uspoređuj sirove frame countove različitih sample rateova.
4. Postavi imenovane početne action/byte budgete, elapsed-time mjerenje i maksimalni broj poteza po krugu. Hard granica ne može prekinuti jedan sinkroni SD poziv: dokumentiraj to i zasebno izmjeri driver timeout. Ne obećavaj real-time gornju granicu dok blocking read postoji.
5. Detail scan napravi nastavivim. Ako se to ne može izolirano napraviti, privremeno odgodi cijeli detail scan dok se reproducira i jasno prikaži pending; nemoj tvrditi da je interaktivni scan tijekom reprodukcije riješen. Puna završnica zahtijeva incremental scan ili dokazanu sigurnu politiku.
6. Koristi jedan filesystem owner. Ne pokreći dva `block_on` posla nad istim volumeom iz različitih taskova.
7. Izmjeri max producer gap, minimalnu zalihu i vrijeme primea. Log formatiranje prebaci iz audio callbacka u niži prioritet uz bounded queue/drop counter.

Testovi s virtualnim satom i sporim fake SourceFiles: A svira dok B ima velik ID3, B radi udaljeni seek, neprekidno browse/detail, obrnuti deckovi, 44.1/48 kHz i promjena pitcha. Zadaj konkretan I/O delay/buffer budget u testu i provjeri da raspoređivanje ne prelazi dostupnu zalihu. Nemoguće fizičke throughput zahtjeve prijavi kao nedostatak kapaciteta, ne kao test koji treba zaobići.

## G. Dokumentacija i održivost — R7

1. U `README.md`, `docs/HANDOFF.md`, `docs/DEVELOPMENT_STATUS.md` postavi jedan datirani aktualni snapshot. Odvojeno navedi shared host, legacy v1.3 device i native >=302.
2. Zastarjele current/next tvrdnje prebaci u označenu povijest ili poveži na povijesni izvještaj. Ne mijenjaj spremljene fizičke logove ni njihove SHA-eve.
3. Sve nove popravke označi HOST_TESTED/BUILD_ONLY/NOT_FLASHED do stvarnog pokusa. Posljednji zabilježeni app ostaje `95d45203...` dok nema novog potvrđenog flash evidencea.
4. Upotrijebi zasebne male testove s lokalnim ownerima gdje je moguće. Globalni test-lock nije zamjena za provjeru međuzadatskih redoslijeda.
5. Za `boxed_copy<T: Clone>` suzi ugovor na stvarno bitwise-copy-safe podatke ili konkretne tipove. Nemoj proizvoljno dodati Copy decoderu s vlasničkim resursima.
6. Stack estimate nazovi procjenom: indirektni pozivi nisu obuhvaćeni. Zadrži postojeće stack guardove; nemoj povisiti limite da build prođe.

## H. Završne offline provjere i handoff

Rust komande pokreći u WSL iz `/mnt/d/AI/M1_PCB` s toolchainom 1.95.0. Logove spremaj na trajan disk, ne samo `/tmp`.

```sh
manifest=firmware-idf/legacy-jc1060/rust-core/Cargo.toml
cargo +1.95.0 fmt --manifest-path "$manifest" -- --check
for feature in idf-media-load idf-controller-audio idf-deck-play idf-deck-mix idf-deck-engine; do
  cargo +1.95.0 test --locked --manifest-path "$manifest" --features "$feature" || exit 1
  cargo +1.95.0 clippy --locked --manifest-path "$manifest" --features "$feature" --tests -- -D warnings || exit 1
done
cargo +1.95.0 test --locked --manifest-path firmware-rust/Cargo.toml -p ruzina-deck -p ruzina-audio-runtime -p ruzina-mixer -p ruzina-uac -p ruzina-media-load -p ruzina-controller-session
```

1. Pokreni i puni target feature loop iz `.github/workflows/legacy-idf.yml`; ne pretpostavi da engine pokriva niže cfg varijante.
2. U IDF Python okruženju s CMake PATH-om: `python -m unittest discover -s firmware-idf/legacy-jc1060/tools/tests -v`. Nedostajući alat je environment failure, ne PASS. Zadnji provjereni Windows Python je `D:/BoardEvidence/JC1060-20261030-COM3/toolchain/python-idf61/Scripts/python.exe`.
3. `python firmware-rust/tools/check_docs.py` i `git diff --check`.
4. Fresh build/package za deck-play, deck-mix i deck-engine pomoću postojećeg `build_legacy.py --help` i zabilježenog toolchaina. Koristi novi output direktorij, ne prepisuj recovery artefakte. Provjeri stack, part-size i native-minimum guardove. Pokreni ostale varijante prema CI matrici.
5. Napravi review finalnog diffa. Ne označi R1–R6 zatvorenima bez testova traženih iznad. Sadašnji testovi možda trebaju prilagodbu novom receipt API-ju; sačuvaj njihov izvorni failure scenario, ne samo ukloni assertion.
6. Ako su commit/push/merge autorizirani u aktivnom zadatku, selektivno stageaj, provjeri staged diff i objavi po smislenu cjelinu. Inače ostavi reviewable lokalne promjene i precizan handoff. Plan sam po sebi ne naređuje publikaciju.
7. PRIJE mergea osvježi PR status. Povijesno #72 ovisi o #71: slijedi dependency order i provjeri retarget/diff nakon mergea #71. Zeleni CI starog SHA-a nije dokaz novog SHA-a.
8. Završni izvještaj mora sadržavati: riješene/otvorene R stavke, test totals i logove, build SHA/package hash, Git status/local-remote SHA kad je objavljeno, stanje ploče i točno prvi sljedeći korak. Ne navodi postotak dovršenosti proizvoda.

## I. Kasnija provjera na ploči — zasebna fizička kampanja

Tek nakon offline popravaka i pripreme točnog paketa, uz fizičku suradnju vlasnika:

1. Provjeri aktualni identitet, firmware, recovery chain i backup prema postojećem postupku. Ne koristi native image na rev103.
2. Kreni s oba decka i poznatim trackovima; odvojeno poslušaj MAIN i PFL. Sačuvaj prihvaćene razine.
3. Provjeri LOAD/PLAY utrke, CUE set/return/preview, rapid seek, EOF, local hot cues, 44.1/48 kHz i dugačke MP3/FLAC datoteke.
4. Zatim odvojeni SD loss i USB hotplug scenariji. Fizičko vađenje i cold cycle radi vlasnik prema pripremljenom monitoru; ne miješaj testove bez oznaka.
5. Mjeri render deadline, producer gap, ring minima, active USB transfers, underruns, memoriju i stack. Prošireni soak vrijedi samo za točan image SHA.
6. Rezultat razlikuje operator listening acceptance, telemetry i neizvršene scenarije. Ako je kandidat eksperimentalan/neprihvatljiv, vrati i provjeri poznati prihvaćeni firmware prema recovery postupku.

## Kratka naredba za sljedećeg agenta

"Pročitaj ovaj plan i povezani review. Implementiraj A–H redom, offline, s testovima uz svaki korak. Ne diraj COM3, native minimum 302, APTA ni izlazne razine. Sačuvaj postojeće lokalne dokumentacijske izmjene. Koristi stvarne load/seek/media generacije i vlasnike, ne simptomatske boolean popravke. Za svaki završeni korak ostavi trajni test/build dokaz i ažuriraj execution log. Fizičku kampanju I pripremi, ali je ne pokreći kao dio offline implementacije."
