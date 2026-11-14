# twinyard (nazwa robocza)

Lekki, stabilny, szybki i nowoczesny dwupanelowy menedżer plików w stylu Total Commandera —
**przede wszystkim na Windows** (Win11), na własny użytek. Ten plik to notatki projektowe dla
Claude/dewelopera: cele, wynik rozpoznania, decyzje i plan.

## Wymagania (od użytkownika)

Must-have:
- Dwa panele, obsługa z klawiatury, skróty jak w TC (F3–F8, Tab, Ins, Ctrl+…).
- Standardowe operacje: kopiuj, przenieś, usuń (kosz / trwale), utwórz katalog/plik, zmień nazwę.
- **FTP / FTPS / SFTP, które działa stabilnie** — główny ból TC i DC: zawieszanie się,
  przerwane transfery, wiszący UI.
- **Import połączeń FTP z Total Commandera** (wygoda przy migracji).
- Porównywarka plików — może być zewnętrzna (WinMerge / Beyond Compare / Meld).
- Konfigurowalny edytor zewnętrzny (F4).
- Tryb ciemny.
- Zakładki (taby w panelach) + ulubione katalogi (hotlist).

Cel jakościowy: nie „retro”, ale też nie ciężkie. Stabilność > liczba funkcji.

## Rozpoznanie istniejących projektów (stan: 2026-11-12)

### getcmdr/cmdr — https://github.com/getcmdr/cmdr
- Rust + **Tauri** (backend) + **Svelte/TypeScript** (frontend). Bardzo duży projekt
  (~13.7k commitów, dziesiątki crate'ów: `cmdr-sftp`, `cmdr-smb`, `cmdr-s3`, `cmdr-webdav`,
  `cmdr-archive`, `cmdr-operations`…). Brak FTP (jest SFTP/SMB/S3/WebDAV).
- **Tylko macOS** — autor wprost pisze, że Windows/Linux „trzeba poczekać”; kod używa
  funkcji specyficznych dla macOS (FSEvents itp.).
- **Licencja: Business Source License 1.1 — NIE open source.**
  - Wolno: czytać, modyfikować, używać prywatnie (jeden użytkownik, niekomercyjnie).
  - Nie wolno: używać w pracy/biznesie bez płatnej licencji ($59).
  - Każdy fork / praca pochodna dziedziczy BSL. Zmiana na AGPL-3.0 dopiero po ~3 latach
    od wydania danej wersji (dla obecnych: 2029-11-06).
  - **Wniosek:** nie kopiujemy z niego kodu. Można podglądać pomysły i architekturę
    (UX, podział na crate'y, kolejka operacji), ale implementacja własna.
- Dowód, że Tauri + web UI może być naprawdę szybkie dla menedżera plików.

### anjunar/rust-commander (RCommander) — https://github.com/anjunar/rust-commander
- Rust + **GTK4** (gtk-rs, sourceview5), v0.4.0, ~110 commitów, jeden autor, od 06.2026.
- Ma: taby, ulubione, kopiuj/przenieś/usuń z postępem i konfliktami, wbudowany edytor,
  podgląd, archiwa (zip/7z/rar/iso), **SFTP przez `ssh2` (libssh2)**. **Brak FTP.**
- Licencja: `MIT` w `Cargo.toml` i README („intended to be MIT”), ale **brak pliku LICENSE**
  w repo + UnRAR ma własną, niewolną licencję. Fragmenty można zapożyczać z atrybucją,
  ale lepiej poprosić autora o dodanie pliku LICENSE.
- Minusy dla nas: GTK4 na Windows = dołączany runtime (dziesiątki MB), wygląd nie-natywny,
  Windows to dla GTK platforma drugiej kategorii; terminal na Windows nie działa.
  `ssh2` na Windows ma problemy z kluczami Ed25519 bez OpenSSL.
- **Wniosek:** dobry materiał referencyjny (MIT), ale nie baza — fork oznaczałby walkę z GTK
  na Windows i dopisywanie FTP od zera.

### Inne, które warto znać (dla porównania UX)
- Double Commander (FPC/Lazarus, GPL) — najbliżej TC, ale toporny; FTP niestabilne (wg użytkownika).
- Multi Commander (Windows, freeware, zamknięty), Directory Opus (płatny, najlepszy na Windows),
  One Commander, Files (C#/WinUI, głównie jeden panel), Far Manager (konsola).
- COSMIC Files (Rust + iced, GPL, Linux) — przykład dojrzałego UI w iced.

## Decyzja: piszemy od nowa, w Rust

Uzasadnienie: żaden istniejący projekt nie spełnia jednocześnie „Windows + licencja + dobre
FTP”. cmdr odpada licencyjnie i platformowo, RCommander technologicznie (GTK) i brakiem FTP.
Rust daje szybkość, brak GC, bezpieczną współbieżność — kluczową dla niezawieszających się
transferów.

### Stos (rekomendacja)
| Warstwa | Wybór | Dlaczego |
| --- | --- | --- |
| Rdzeń | Rust (workspace, kilka crate'ów) | wydajność, stabilność, async |
| UI | **Tauri 2 + Svelte 5 + TypeScript** | WebView2 jest już w Win11 → mały binarny (~10–15 MB); łatwy nowoczesny wygląd, tryb ciemny, animacje; sprawdzone w cmdr |
| Async | tokio | transfery, timeouty, anulowanie |
| FTP/FTPS | `suppaftp` (async, rustls) | aktywnie rozwijany, PASV/EPSV, TLS |
| SFTP | `russh` + `russh-sftp` (czysty Rust) | brak libssh2/OpenSSL, Ed25519 od ręki, async |
| Lokalny FS | `std::fs` + `windows` crate (CopyFileEx z postępem, kosz przez `IFileOperation`/`trash`) | natywne, szybkie kopiowanie na Windows |
| Watcher | `notify` | odświeżanie paneli |
| Konfiguracja | TOML (`serde`, `toml`) w `%APPDATA%\twinyard\` | czytelna, ręcznie edytowalna |
| Hasła | Windows Credential Manager (`keyring`) | nie trzymamy haseł w plikach |

Alternatywy rozważone: **Slint** (lżejszy, natywny rendering, ale mniej elastyczny dla
złożonego UI), **egui** (bardzo szybki, ale wygląd „narzędziowy”, słabsza dostępność),
**iced** (dojrzewa, ale wciąż mniej wygodny niż web), **GTK4** (patrz wyżej),
**WinUI 3 / C#** (natywne, ale cięższe wdrożenie i nie Rust). Jeśli Tauri okaże się za ciężkie
(RAM ~100+ MB przez WebView2) — plan B: Slint, rdzeń zostaje bez zmian.

Ryzyka Tauri do sprawdzenia w prototypie:
- Płynność listy przy 100k+ plików (wirtualizacja, paczkowanie danych z backendu, ikony z cache).
- Fokus i skróty klawiaturowe (przechwycenie F-klawiszy przed WebView, Alt bez menu przeglądarki).
- Drag & drop **do** Explorera (wymaga natywnego kodu / pluginu), menu kontekstowe Explorera
  (`IContextMenu` przez Win32 — do zrobienia w Rust).

## Architektura (zarys)

```
crates/
  core/        lokalny FS: listing (sort naturalny PL), dyski, Win32 copy/move, kosz,
               silnik operacji (ops.rs: Host trait, konflikty, błędy), config, ulubione
  remote/      FTP/FTPS: path.rs (ftp://<id>/…), conn.rs (connections.json + przypięte certy),
               creds.rs (Credential Manager), tls.rs (weryfikator z pinningiem), ftp.rs (sesja),
               manager.rs (sesje listujące, reconnect, keepalive), engine.rs (transfery), tc.rs (import)
app/           Tauri: komendy IPC (lib.rs, jobs.rs, ftp.rs) + frontend Svelte (app/src)
```
(Stan faktyczny. Pierwotny szkic zakładał osobne crate'y local/ops/config/tcimport/sftp — SFTP
dojdzie jako kolejny moduł w `remote`.)

```
```

Zasady:
- **Żadne I/O w wątku UI.** Każda operacja = zadanie w kolejce z postępem, anulowaniem i logiem.
- Panel zdalny jest „tylko widokiem” — rozłączenie nie zawiesza UI, tylko pokazuje stan
  i oferuje ponowne połączenie.

### Stabilne FTP/SFTP — konkretne mechanizmy (to ma być przewaga nad TC/DC)
- Twarde timeouty na connect / każdą komendę / brak postępu transferu (watchdog).
- Osobne połączenie do listowania i osobne do transferów (TC blokuje się na jednym).
- Keepalive (NOOP dla FTP, SSH keepalive), automatyczne ponowne połączenie.
- Retry z backoffem per plik; wznawianie (FTP `REST`, SFTP offset) zamiast zaczynania od nowa.
- Kolejka transferów w tle, możliwość pauzy/anulowania bez zabijania sesji.
- Weryfikacja rozmiaru po transferze; tryb pasywny domyślnie, EPSV, obsługa NAT
  (ignorowanie prywatnego IP z odpowiedzi PASV).
- Wykrywanie kodowania nazw (UTF-8 / `OPTS UTF8 ON` / fallback CP1250).
- Szczegółowy log sesji do podglądu (jak w TC, ale z możliwością kopiowania).

### Import z Total Commandera
- Lokalizacja: `%APPDATA%\GHISLER\` (lub katalog TC przy instalacji portable / wpisy w `wincmd.ini`).
- Użytkownik korzysta **wyłącznie z wbudowanego FTP/FTPS TC** (nie z wtyczki SFTP).
- Zaimportowane hasła trafiają do Windows Credential Manager, nie do naszego configu.
- `wincmd.ini` → `[DirMenu]` (hotlist katalogów → nasze ulubione); taby ewentualnie później.

#### Format `wcx_ftp.ini` (zweryfikowany na prawdziwym pliku użytkownika, 2026-11-12)
Prawdziwy plik leży lokalnie w katalogu projektu, jest w `.gitignore` i **nie wolno go
commitować ani cytować** (zawiera odwracalnie zaszyfrowane hasło i dane firmowe). Do testów
robimy zanonimizowany fixture w `crates/tcimport/tests/fixtures/`.

| Sekcja / klucz | Znaczenie | Mapowanie u nas |
| --- | --- | --- |
| `[connections]` `1=Nazwa`, `2=…` | lista połączeń (kolejność); nazwy mogą mieć **spacje na końcu** → trim | lista połączeń |
| `[connections]` `default=Nazwa` | ostatnio/domyślnie wybrane | zaznaczenie w menedżerze |
| `[OldConnections]` | historia szybkich połączeń | pomijamy / opcjonalnie |
| `[default]` | ustawienia domyślne nowych połączeń (`pasvmode=1`) | domyślne |
| `[General]` `ConnectRetries`, `WaitDelay` | liczba prób połączenia, odstęp (s) | polityka retry |
| `[General]` `AutoResume`, `transfermode=I`, `ModeZ=1` | wznawianie, binarny, kompresja MODE Z | ustawienia transferu |
| `[<Nazwa>]` — sekcja nazwana jak połączenie (nazwa może zawierać spacje i `host:port`) | | |
| `host` | `host`, `host:port` albo **`ftps://host:990`** (schemat w polu host!) | parsowanie schemat/host/port |
| `username`, `password` | hasło: hex, szyfrowanie TC (puste = pytaj) | dekoder TC → Credential Manager |
| `pasvmode` | 1 = pasywny | tryb |
| `keepalive=30,PWD` / `90,NOOP` | interwał (s) + komenda | keepalive per połączenie |
| `CacheDirs=0`, `MLSD=-1` | bez cache katalogów; MLSD wyłączone (fallback na LIST) | flagi |
| `[CertSHA1hashes]` / `[CertSHA256hashes]` `host:port=AA:BB:…` | **zaufane (przypięte) certyfikaty** | magazyn zaufanych certyfikatów |
| `[CertAlias]` `ip=*.domena` | akceptowana niezgodność nazwy w certyfikacie dla IP | wyjątki weryfikacji nazwy |

Uwagi implementacyjne:
- Schematy w `host`: `ftps://` (implicit TLS, 990); explicit TLS (AUTH TLS) — sprawdzić, jak TC
  to zapisuje (prawdopodobnie osobny klucz/flaga); brak schematu = zwykłe FTP.
- Algorytm haseł TC (bez hasła głównego) jest publicznie znany (m.in. implementacja w Double
  Commanderze) — zaimplementować od nowa z testami na wygenerowanych danych. Hasła z hasłem
  głównym TC: import bez hasła + pytanie przy pierwszym połączeniu.

### Integracje zewnętrzne (konfigurowalne szablony poleceń)
```toml
[tools]
editor  = '"C:\Program Files\Notepad++\notepad++.exe" "%f"'
viewer  = "builtin"
diff    = '"C:\Program Files\WinMerge\WinMergeU.exe" /e /u "%l" "%r"'
```
Dla plików zdalnych: pobranie do temp → edycja → wykrycie zapisu → pytanie o upload.

## Uruchamianie
```powershell
cd app
npm install
npx tauri dev          # tryb deweloperski (hot reload UI)
npx tauri build        # instalator NSIS w target/release/bundle/nsis
cargo test -p twinyard-core
```
Struktura: `crates/core` (FS, sortowanie naturalne, dyski — bez UI) · `app/src-tauri` (komendy
Tauri) · `app/src` (Svelte: `App.svelte`, `lib/Panel.svelte`, `lib/panelState.svelte.ts`).

Uwaga dla Claude: na Windows PowerShell 5.1 `Get-Content`/`Set-Content` psuje UTF-8 (polskie
znaki) — pliki edytować narzędziem Edit, nie zamianą w PowerShellu. Nie nazywać plików różniących
się tylko wielkością liter (`Panel.svelte` vs `panel.svelte.ts` — kolizja na NTFS).

## Stan prototypu (krok 1, 2026-11-12)
Zrobione: dwa panele z wirtualizowaną listą (renderuje tylko widoczne wiersze), dyski
(`GetLogicalDrives`, bez budzenia napędów), nawigacja TC (strzałki, PgUp/PgDn, Home/End, Enter,
Backspace, Ctrl+PgUp/PgDn, Tab), zaznaczanie (Ins, Spacja, Shift+strzałki, prawy klik, num `*`/`+`/`-`,
Ctrl+A), sortowanie (klik w nagłówek, Ctrl+F3–F6; katalogi zawsze na górze), szybkie wyszukiwanie
(pisanie liter), ukryte pliki (Ctrl+H), zamiana paneli (Ctrl+U), motyw systemowy/ciemny/jasny
(łącznie z paskiem tytułu), zapamiętywanie ścieżek, test wydajności (Ctrl+Alt+F = 100k,
Ctrl+Alt+M = 1M sztucznych pozycji; czasy backend · IPC · sort w stopce panelu).

Decyzje techniczne:
- Listing przesyłany **kolumnowo** (`names[]`, `sizes[]`…), nie jako tablica obiektów.
- Sortowanie naturalne liczone **w Rust** raz na listing (`nameRank`), w JS już tylko porównania
  liczb. Pomiar: `Intl.Collator` na 100k nazw = ~430 ms, stąd ta decyzja.
- Każde I/O w `spawn_blocking`; spóźnione odpowiedzi (stara ścieżka) są odrzucane.
- Skróty przeglądarki (F5, Ctrl+F, Ctrl+P, Alt+←…) blokowane w JS. Tauri nie wystawia
  `AreBrowserAcceleratorKeysEnabled` — jeśli coś się przebije, wyłączyć natywnie przez
  `with_webview` + `webview2-com` (`ICoreWebView2Settings3`).

Do weryfikacji przez użytkownika: płynność przewijania 100k/1M, czy jakiś skrót przeglądarki
się przebija, wygląd motywów.

## Stan kroków 2–4 (2026-11-12, praca autonomiczna — czeka na przeklikanie przez użytkownika)
- **Własny pasek tytułu** (`decorations: false`): suwaki = ustawienia, słońce/księżyc = motyw
  (domyślnie systemowy, klik przełącza na stałe), ✥ = przeciąganie okna, min/max/zamknij
  w stylu Win11. Puste miejsce paska też przeciąga, dwuklik maksymalizuje.
- **Operacje** (`crates/core/src/ops.rs`, testy jednostkowe): kopiuj/przenieś/kosz/usuń trwale.
  `CopyFileExW` z postępem i przerwaniem w trakcie pliku; przeniesienie na tym samym wolumenie =
  `MoveFileExW` (natychmiast), między wolumenami = kopia + usunięcie źródła dopiero po sukcesie;
  konflikty: nadpisz / wszystkie / starsze / pomiń / pomiń wszystkie / zachowaj oba / anuluj;
  błędy: ponów / pomiń / pomiń wszystkie / anuluj; ochrona przed kopiowaniem katalogu do siebie;
  junction/symlink nie są przechodzone (kopiowanie je pomija, usuwanie usuwa sam link).
  Kosz przez crate `trash` (`FOF_WANTNUKEWARNING`: czego kosz nie przyjmie → błąd, nie ciche
  usunięcie).
- **Zadania w tle** (`app/src-tauri/src/jobs.rs`): wątek na zadanie, zdarzenia `job-progress`
  (≤ co 80 ms) / `job-ask` / `job-done`, pytania blokują wątek do `job_answer`, `job_cancel`
  odblokowuje. Karty postępu w prawym dolnym rogu (prędkość, ETA, lista błędów).
- **Zakładki**: Ctrl+T, Ctrl+W, Ctrl+Tab / Ctrl+Shift+Tab, Ctrl+↑ = katalog pod kursorem w nowej
  zakładce, środkowy klik zamyka; zapamiętywane między uruchomieniami.
- **Konfiguracja** `%APPDATA%\twinyard\config.toml` (tworzona przy pierwszym użyciu, z wykrytymi
  narzędziami; czytana przy każdym użyciu): `editor` (F4), `viewer` (F3), `diff` (Ctrl+F2).
  Na tym komputerze wykryte VS Code (edytor + `--diff`).
- **Ulubione** Ctrl+D: `%APPDATA%\twinyard\favorites.json` (zapis atomowy). Import z TC
  `[DirMenu]` — gdy użytkownik dostarczy `wincmd.ini` (na tym komputerze TC nie ma).

## Stan kroku 5 — FTP/FTPS (2026-11-12)
- Ścieżki panelu: `ftp://<id-połączenia>/abs/ścieżka`; `list_dir`/`parent_dir`/`create_dir`/`rename_path`/
  `open_path`/`run_tool` w `app/src-tauri/src/lib.rs` rozpoznają je i kierują do `twinyard_remote`.
- Zadania (F5/F6/F8) z udziałem FTP idą przez `twinyard_remote::engine::run` (ten sam `Host`, te same
  okna konfliktów/błędów; nowa akcja konfliktu **Wznów** gdy cel jest krótszy).
- Stabilność: każde zadanie ma własne połączenie; timeouty connect 15 s / I/O 30 s / całe łączenie 45 s
  (wątek-strażnik); zerwanie → 3 automatyczne ponowienia (1/3/6 s) z wznowieniem (REST / APPE);
  weryfikacja rozmiaru po każdym pliku; przedwczesny koniec danych → natychmiastowe wznowienie (bez
  czekania na odpowiedź martwego serwera); sesja listująca: NOOP przed użyciem po 20 s bezczynności,
  keepalive co 45 s, zamknięcie po 20 min; PASV z obejściem NAT.
- TLS: rustls + weryfikator Windows (`rustls-platform-verifier`); odrzucony certyfikat → okno z SHA-256
  → „Zaufaj” zapisuje pin w `connections.json` (też piny SHA-1/256 z TC). Implicit (990) wysyła ręcznie
  `PBSZ 0`/`PROT P` (suppaftp robi to tylko dla AUTH TLS). Puste pliki przez TLS: patrz komentarz
  w `engine.rs::put_file`.
- `ftps://` w TC = **AUTH TLS (jawne) na podanym porcie**, także 990 (zweryfikowane na serwerze
  produkcyjnym: port 990, powitanie „220” otwartym tekstem). Import mapuje na `explicit`;
  połączenie skonfigurowane jako `implicit` najpierw sprawdza (≤1,5 s, wynik w pamięci), czy serwer
  wita się tekstem, i wtedy samo przechodzi na AUTH TLS. Uwaga: implicit connect w suppaftp nie ma
  timeoutu na handshake — stąd sprawdzenie *przed* próbą.
- Hasła: `data\passwords.json` zaszyfrowane DPAPI (wcześniej Menedżer poświadczeń — migracja automatyczna). Import z TC nie dekoduje
  haseł TC — program pyta przy pierwszym połączeniu.
- F3/F4/porównanie/Enter na pliku z FTP: pobranie do `%TEMP%\twinyard\<id>\…` — **zmiany NIE wracają
  na serwer** — wyjątek: **F4** (`edits.rs`): kopia jest obserwowana (co 1 s), po zapisie w edytorze
  pytanie „Wysłać na serwer?” (+ „wysyłaj automatycznie” dla tego pliku); upload przez silnik
  (ponowienia/wznawianie), potem odświeżenie paneli z tym katalogiem.
- Brakuje: kopiowanie serwer↔serwer, Shift+F4 na FTP, ustawianie daty po uploadzie (MFMT), kodowanie
  nazw innych niż UTF-8 (CP1250), log sesji, SFTP.

### Testy FTP (lokalny serwer)
`crates/remote/tests/live.rs` — FTP, FTPS jawne, niejawne z pinningiem, zrywanie połączenia w połowie
pliku. Serwer testowy: pyftpdlib (skrypt w scratchpadzie sesji; porty 2121 FTP, 2122 AUTH TLS,
2990 implicit, 2123 „zrywający”; user `test`/`pass`; certyfikat MUSI być X.509 v3 — webpki odrzuca v1).
Uruchomienie: `set TWINYARD_FTP_TEST=1` i `cargo test -p twinyard-remote --test live -- --test-threads=1`.
Bez zmiennej testy są no-op. Import z prawdziwego pliku TC (wypisuje tylko liczby):
`set TWINYARD_TC_INI=ścieżka` + `cargo test -p twinyard-remote real_file -- --nocapture`.

## Przenośność (wymaganie użytkownika, 2026-11-12)
Aplikacja jest **w pełni portable**: wszystkie dane w `data\` obok `twinyard.exe`
(`core::config::data_dir()`): `config.toml`, `favorites.json`, `connections.json`,
`passwords.json` (hasła FTP zaszyfrowane DPAPI dla bieżącego konta Windows — na innym PC/koncie
nie dają się odszyfrować i program pyta ponownie), `logs\ftp.log`, `webview\` (profil WebView2:
localStorage z zakładkami i motywem — przez `WEBVIEW2_USER_DATA_FOLDER` ustawiane w `main.rs`).
Build deweloperski (`target\debug`) używa `<repo>\data\` (w `.gitignore`). Fallback na
`%APPDATA%\Twinyard` tylko gdy katalog exe jest tylko do odczytu. (Migracje ze starych lokalizacji —
`%APPDATA%\filecmd`, Menedżer poświadczeń, klucze localStorage `filecmd.` — usunięte 2026-11-14 przed
publikacją 0.9.0.) Testy: `TWINYARD_DATA` nadpisuje katalog. **Nie zapisywać niczego poza `data\`** (wyjątek:
`%TEMP%\twinyard` na kopie plików z FTP).

### Tryb zgodności TLS (SChannel)
Jeden z serwerów produkcyjnych (FTPS, AUTH TLS na porcie 990) obsługuje tylko statyczną wymianę kluczy RSA
(`AES256-GCM-SHA384`), której rustls nie ma. `tlsio.rs`: jeden typ strumienia dla suppaftp z dwoma
silnikami; po odmowie handshake'u (HandshakeFailure itp.) sesja automatycznie łączy się przez
SChannel (native-tls), pamiętając to dla hosta do końca działania programu. Certyfikat w trybie
SChannel sprawdza ten sam `PinVerifier` (pin albo łańcuch Windows). Test: serwer 2124 w live.rs.
Log FTP: `data\logs\ftp.log` (komendy/odpowiedzi, PASS zamaskowane), przycisk „Log” w Ctrl+F.

## Stan po sesji 2026-11-12 (popołudnie)
Zrobione: wbudowany podgląd F3 (tekst/hex/obrazy, szukanie; `Viewer.svelte`) i porównywarka Ctrl+F2
(`DiffView.svelte`, Myers w `diff.ts`; `config.toml` → `viewer`/`diff` = 'builtin'); dekodowanie tekstu
z CP1250 (`core/text.rs`); pasek narzędzi w pasku tytułu; F1 okno skrótów (`shortcuts.ts` — trzymać
w zgodzie z `onKey` w App.svelte); historia katalogów (Alt+←/→, boczne przyciski myszy, Alt+↓ lista);
szybki filtr Ctrl+S; Shift+F5 kopia tutaj; terminal w katalogu; zakładki FTP nie są przywracane;
wyszukiwanie Alt+F7 (`core/search.rs`, `[search] text_ext`; streaming `search-batch`).
**Niesprawdzone w oknie:** okno wyszukiwania (Windows zablokował przeniesienie okna na wierzch
podczas testu — core ma testy).

Dalej (uzgodnione z użytkownikiem): porównanie i synchronizacja katalogów (też lokalny ↔ FTP),
MFMT (data po uploadzie), nazwy CP1250 na starych FTP, paczka portable (release exe + `data\`).
Czeka na użytkownika: test połączenia z serwerem produkcyjnym (tryb SChannel), `wincmd.ini` do hotlisty.
Do testów UI: `scratchpad\safekeys.ps1` (wysyła klawisze tylko gdy twinyard na wierzchu) — nigdy
zwykłe SendKeys bez sprawdzenia fokusu.

## Stan po sesji autonomicznej 2026-11-12 (14:41–15:00)
- ✅ Alt+F7 sprawdzone w oknie; poprawki: fokus na polu nazwy (Modal: `[data-autofocus]` ma pierwszeństwo),
  wyścig „wyniki przed id” (bufor `early` w Search.svelte), walidacja katalogu.
- ✅ Porównanie/synchronizacja katalogów Shift+F2 (`dircmp.rs` + `SyncView.svelte`): lokalny↔lokalny,
  lokalny↔FTP, FTP↔FTP; kierunki →/←/pomiń; kopiowanie sekwencyjnie przez `jobs.run()` (czeka na koniec
  zadania), grupami wg katalogu docelowego; katalogi bez dostępu pomijane i liczone.
- ✅ Warstwy: dialogi (z-index 90) i karty zadań (85) nad widokami pełnoekranowymi (70).
- ✅ Pasek tytułu: `.app` ma `grid-template-columns: minmax(0,1fr)` — długa ścieżka nie wypycha przycisków.
- ✅ MFMT po uploadzie (`Session::set_mtime`, `utc_stamp`), sprawdzone w testach live.
- ✅ Kodowanie nazw na FTP per połączenie: auto (UTF8 z FEAT albo wykrywanie UTF-8/CP1250, `names.rs`),
  UTF-8, Windows-1250. Test live na serwerze 2126 (pyftpdlib `encoding="cp1250"`).
- ✅ Paczka portable: `scripts\build-portable.ps1 [-WithData]` → `dist-portable\twinyard\twinyard.exe`
  (8 MB, release, bez instalatora); dane w `dist-portable\twinyard\data\` (w .gitignore).

## Stan 2026-11-12 wieczór (po uwagach użytkownika — korzysta już na co dzień)
- ✅ Ikony typów plików (`fileIcons.ts`: rozszerzenie → rodzaj → kolorowa ikona 16 px).
- ✅ Okno konfiguracji (zębatka, `Settings.svelte`): widoczność przycisków paska (localStorage
  `toolbarHidden`, czyli w data\webview — portable), motyw, otwieranie config.toml.
- ✅ ZIP (`core/archive/`): ścieżka `C:\x\a.zip\wewn` listuje wnętrze; Enter na .zip wchodzi;
  F5 z archiwum = rozpakowanie (konflikty, postęp, daty DOS↔lokalne przez Win32); F3/F4/Enter na
  pliku w archiwum = wypakowanie do %TEMP%; Alt+F5 = pakowanie (`OpKind::Pack`, deflate, .zip.part →
  rename). Porównanie katalogów działa też katalog↔zip. (Zapis do ZIP, 7z i RAR — niżej.)
- ✅ Otwieranie plików (Enter): `win::shell_open` = ShellExecuteEx bez okien błędów; brak działającego
  skojarzenia → edytor (rozszerzenia z `[search] text_ext`) albo „Otwórz za pomocą…”. (U użytkownika
  .txt ma zepsute skojarzenie → wcześniej „Nie można odnaleźć aplikacji”.)
- Pominięte na życzenie: import hotlisty TC.
- ✅ Przeciąganie (App.svelte `startDrag`, Panel `rowDown`): między panelami / na katalog (Shift = przenieś),
  z Eksploratora (Tauri onDragDropEvent), poza okno → OLE (`dnd.rs`, crate `drag`). Niesprawdzone w oknie.
- ✅ Menu kontekstowe Eksploratora (`ctxmenu.rs`, IContextMenu2/3 + subclass na czas menu): prawy klik,
  klawisz Menu, Shift+F10. Prawy klik już NIE zaznacza (Ctrl+klik / Insert). Niesprawdzone w oknie.
- ✅ `\\serwer` bez udziału = lista udziałów (`core/unc.rs`, NetShareEnum); „..” z `\\serwer\udział` → `\\serwer`.
- ✅ Przyciski dysków: ten sam dysk co drugi panel → jego katalog; inaczej ostatni katalog na dysku (jak TC).
- ✅ Shift+F4 na FTP (`edit_new_remote`), FTP serwer↔serwer i kopia w obrębie serwera (`engine::relay`).
- ✅ Wyszukiwanie na FTP (`remote/search.rs`, tekst tylko w plikach tekstowych ≤ 2 MB).
- ✅ Kodowanie nazw: serwery „UTF8 w FEAT, ale 501 Please CLNT first” (np. Serv-U) — CLNT + ponowne
  OPTS; jeśli UTF-8 się nie włączy, tryb wykrywania wysyła nazwy w CP1250. Sprawdzone przez użytkownika
  na dwóch serwerach produkcyjnych.
- ✅ Bez zamrażania UI przy martwych ścieżkach sieciowych (obserwator katalogów w osobnym wątku).
- Edycja skrótów — na razie NIEPOTRZEBNA (decyzja użytkownika).
- Testy UI: użytkownik pracuje na tym komputerze — przed `safekeys.ps1` upewnić się, że nie jest
  aktywny; NIE naciskać Enter na plikach w testach (otwiera programy / okna systemowe).

## Archiwa: zapis do ZIP, odczyt 7z i RAR (sesja 2026-11-12, wieczór)
Moduł `crates/core/src/archive/`: `mod.rs` (ścieżki, listowanie, zadania), `zipfmt.rs`, `sevenz.rs`, `rar.rs`,
testy w `tests.rs` (`cargo test -p twinyard-core archive`; zgodność z .NET: `cargo test -p twinyard-core
interop -- --ignored`).
- ✅ **ZIP — zapis:** F5/F6/przeciąganie do archiwum (dopisanie, konflikty jak przy kopiowaniu, bez
  rozróżniania wielkości liter), F8 (zawsze trwale — w archiwum nie ma kosza), F2/Shift+F6 (pliki i katalogi),
  F7, F6 z archiwum (rozpakuj + usuń z archiwum), archiwum↔archiwum (przez katalog tymczasowy), F4 na
  pliku w ZIP = jak na FTP: po zapisie w edytorze pytanie „Zaktualizować archiwum?”.
  Każda zmiana **przepisuje całe archiwum** do `<nazwa>.zip.part` — pozostałe wpisy kopiowane surowo
  (`raw_copy_file`, bez rekompresji, nazwy w starych stronach kodowych bez zmian), komentarz archiwum
  zachowany — i dopiero na końcu podmiana (`MoveFileEx`). Błąd lub anulowanie = oryginał nietknięty.
  Usunięcie ostatnich plików z folderu zostawia folder (jawny wpis `folder/`), żeby panel nie stracił katalogu.
- ✅ **7z** (`sevenz-rust2`, czysty Rust, Apache-2.0; LZMA/LZMA2/BZip2/Deflate/PPMd/BCJ) i **RAR**
  (biblioteka UnRAR 7.1 przez `unrar_sys`, własny cienki wrapper FFI w `rar.rs` — dane strumieniowo przez
  callback, więc postęp i anulowanie w trakcie dużego pliku): przeglądanie, F5 = rozpakuj, F3/F4/Enter.
  **Tylko do odczytu** (F8/F6/F7 w nich = komunikat). RAR wieloczęściowe: otwierać pierwszą część.
- ✅ **Archiwa z hasłem — odczyt** (ZIP ZipCrypto + AES, 7z AES-256, RAR 4/5, też z zaszyfrowanymi nazwami
  w 7z/RAR). Tworzenia/zapisu z hasłem NIE ma (decyzja użytkownika); dopisanie do zaszyfrowanego ZIP zostawia
  stare wpisy zaszyfrowane, nowe bez hasła. Mechanizm: formaty zwracają błąd `NeedPassword{wrong}`,
  `archive::decode`/`entries` pytają `Host::password` i ponawiają tylko to, czego jeszcze nie rozpakowano.
  W zadaniach (F5) pyta okno w `JobsView` (`job-ask` kind `password`); poza zadaniami (wejście do archiwum,
  F3/F4/Enter) backend zwraca tekst „Wymagane|Złe hasło do archiwum: <ścieżka>”, a `call()` w `api.ts`
  pokazuje okno, wysyła `archive_password` i ponawia wywołanie. Hasła **tylko w pamięci** do zamknięcia
  programu (nigdy na dysku). Złe hasło bez kontroli (ZipCrypto, RAR 4, 7z) wykrywane po błędzie CRC/danych —
  wtedy ponowne pytanie „Złe hasło”; uszkodzone zaszyfrowane archiwum z dobrym hasłem też tak wygląda
  (Anuluj przerywa). Testy: `zip_aes_password`, `sevenz_password`, `rar_passwords` (ZipCrypto nie da się
  utworzyć w teście — `zip` nie wystawia tego publicznie; odczyt idzie tą samą drogą co AES).
- **Licencja UnRAR:** kod wolno używać za darmo do obsługi (rozpakowywania) RAR w dowolnym programie,
  nie wolno go użyć do stworzenia archiwizatora RAR. Wymóg: pełny akapit licencji w dokumentacji i
  komentarzach — jest w `THIRD-PARTY-NOTICES.md` (kopiowany do paczki portable) i w `rar.rs`.
  Odrzucone: `unrar-rs` (brak licencji), crate `unrar` (brak strumieniowania z postępem).
- Poprawki przy okazji: F5 na samym pliku .zip w zwykłym panelu był brany za „wnętrze archiwum” i nic nie
  kopiował (`is_inside` = tylko ścieżki *wewnątrz* albo `a.zip\` jako cel); F3 na pliku .zip; przeciąganie
  z archiwum poza okno = komunikat (Eksplorator tych ścieżek nie widzi); FTP↔archiwum = komunikat.
- Fixtures RAR: `crates/core/tests/fixtures/*.rar` (z crate'a `unrar`, MIT/Apache).
- **Niesprawdzone w oknie** (czeka na użytkownika): wszystkie powyższe w UI — rdzeń ma testy.

## Język interfejsu PL/EN + poprawki z przeglądu (sesja 2026-11-12, noc)
- ✅ **Język**: Konfiguracja (zębatka) → „Język / Language”; zmiana działa od razu, zapis w `data\state.json`
  (klucz `lang`); bez ustawienia decyduje język systemu. Frontend: teksty zostają w kodzie po polsku i są
  kluczami — `t("Nowy katalog")`, `t("Kopiuj {0}", x)`, `tn(n, "plik|pliki|plików")` (`app/src/lib/i18n.svelte.ts`),
  angielskie w `i18n.en.ts`. **Po dodaniu tekstu:** `node scripts/check-i18n.mjs` (braki / nieużywane).
  Backend: `twinyard_core::i18n::tr("pl", "en")` i `trf!("pl {x}", "en {x}")`, język ustawia komenda `set_lang`.
  Skróty przycisków w oknach: `&` w etykiecie (`"&Nadpisz"` / `"&Overwrite"`), każda wersja ma swoje litery.
  Tekst „Wymagane|Złe hasło do archiwum: …” to protokół backend→UI — zostaje po polsku, UI tłumaczy przy wyświetlaniu.
  Identyfikatory przycisków paska (`toolbarHidden`) = polskie nazwy (zgodność z zapisanym stanem).
- ✅ F5/F6/Alt+F5: cel wpisany bez dysku („backup”, „..\x”) jest liczony od katalogu źródłowego (wcześniej od
  katalogu programu!). ✅ Zamykanie okna przy trwających zadaniach pyta o potwierdzenie (`setCloseGuard`).
  ✅ Przycisk „Dokończ (wznów)” w oknie konfliktu, gdy cel na FTP/dysku jest krótszy (backend to umiał, UI nie).
- **Niesprawdzone w oknie** (typy, build, testy Rust — OK). Paczka portable nieprzebudowana.

## Luki i funkcje z przeglądu (sesja 2026-11-13)
- ✅ `%TEMP%\twinyard`: przy starcie usuwane kopie starsze niż 3 dni (wg daty *utworzenia* — pobrane pliki mają
  datę z serwera), puste katalogi też (`housekeeping.rs`).
- ✅ Okno: rozmiar/pozycja/maksymalizacja w `data\window.json` (`winstate.rs`; okno startuje ukryte
  — `"visible": false` w tauri.conf.json — i pokazuje się po odtworzeniu; zapis pomijany, gdy monitora już nie ma).
- ✅ Jedna instancja na katalog `data\` (mutex `Local\twinyard-<hash>` + `data\instance.pid`; drugie uruchomienie
  wyciąga okno pierwszego). Build deweloperski i portable mają różne `data\` → mogą działać równolegle.
- ✅ F3/F4/Enter na pliku z FTP ≥ 1 MB: pobieranie jako zwykłe zadanie (postęp, anulowanie, ponowienia,
  wznawianie) do tej samej ścieżki co `fetch_temp`; zadania z `keepPanels` nie czyszczą zaznaczeń.
- ✅ Stopka panelu: wolne / całkowite miejsce (też UNC i archiwa = wolumin archiwum).
- ✅ Spacja na katalogu = zaznacz + policz rozmiar (lokalnie, FTP, archiwum); Alt+Shift+Enter = wszystkie;
  Esc przerywa (`dir_size_cancel`, numer generacji). Rozmiary liczą się w podsumowaniu stopki.
- ✅ Num + / Num − = (od)znacz maską (pliki; `*\` = katalogi), Ctrl+Num − = odznacz wszystko, Ctrl+A bez zmian.
- ✅ Schowek plików Ctrl+C / Ctrl+X / Ctrl+V (CF_HDROP + „Preferred DropEffect” — wspólny z Eksploratorem;
  wklejanie też na FTP i do ZIP; po wklejeniu wyciętych schowek czyszczony), Ctrl+Shift+C ścieżki,
  Ctrl+Alt+C nazwy jako tekst (`core/clipboard.rs`). Niesprawdzone w oknie.
- ✅ Ctrl+← / Ctrl+→: katalog spod kursora (albo bieżący) w lewym / prawym panelu.
- ✅ Ctrl+M wielokrotna zmiana nazw (`MultiRename.svelte`): maska `[N] [E] [C] [P] [Y] [M] [D]`, znajdź/zamień
  (regex z `$1`), wielkość liter, licznik, podgląd z konfliktami; zamiany a↔b / przesunięcia licznika
  dwufazowo przez nazwy tymczasowe; działa przez `rename_path` (lokalnie, FTP, ZIP).
- ✅ Shift+F2 „po zawartości”: pliki o równym rozmiarze porównywane bajt po bajcie (tylko dysk↔dysk).
- ✅ **Przeklikane w oknie** (release z danymi deweloperskimi): wolne miejsce, Spacja/Alt+Shift+Enter, maska Num +,
  Ctrl+C → schowek (Windows widzi FileDrop) → Ctrl+V, Ctrl+M (też przesunięcie licznika dwufazowo), Ctrl+→,
  język EN, F3 na 8 MB z FTP (karta postępu, kopia identyczna), pytanie przy zamykaniu w trakcie zadania,
  druga instancja, zapamiętanie okna. Poprawki po testach: zapis okna bez przesuwania, rozmiar wewnętrzny
  (okno rosło o ramkę przy każdym starcie), BOM w config.toml / state.json / favorites / connections nie psuje
  wczytywania, maska w Ctrl+M zaznaczona przy otwarciu, „instancja” bez widocznego okna nie blokuje startu.
- Zamykanie: proces znika po ~0,5 s (też z sesją FTP i w trakcie pobierania po potwierdzeniu); nic nie wisi.
- Testy UI: połączenie `local-test` w `<repo>\data\connections.json` = lokalny pyftpdlib (127.0.0.1:2131,
  test/pass, venv w scratchpadzie sesji). Numpad przez SendKeys nie ma `e.code` — wysyłać z kodem skanowania.
- **Uwaga dla Claude:** heredoc w Bash potrafi zjeść backslash (`/\\+$/` → `/\+$/`) — łatki z `\` pisać
  narzędziem Write.
- Dalej (pomysły): cofanie ostatniej operacji (np. zmiany nazw), Alt+F7 wewnątrz archiwów, pakowanie do 7z, .tar/.gz.

## Archiwa .tar / .tar.gz / .tgz / .gz (sesja 2026-11-13, rano)
- ✅ Tylko do odczytu, jak 7z/RAR: Enter wchodzi, F3/F4 podgląd, F5 wypakowuje (`archive/tarfmt.rs`, crate'y
  `tar` + `flate2`). Pojedynczy `x.gz` = archiwum z jednym plikiem `x` (rozmiar z końcówki gzip, mod 4 GiB).
- Skompresowany tar nie ma spisu treści: lista = przeczytanie całości, więc jest pamiętana (4 ostatnie archiwa,
  klucz: ścieżka + rozmiar + data) — wchodzenie w podkatalogi nie rozpakowuje od nowa. Wypakowanie kończy
  czytanie po ostatnim potrzebnym pliku. Zwykły .tar przeskakuje dane (seek). Linki i urządzenia pomijane.
- Testy: tar/tar.gz/tgz z polskimi nazwami, ścieżką >100 znaków (GNU long name), linkiem; cache; .gz i uszkodzony .gz.
- ✅ Przeklikane w oknie na archiwach z GNU tar (Git Bash) i Windowsowego `tar.exe`: wejście, polskie nazwy,
  F5 całego folderu (hash i daty zgodne), F3, F8 odmawia (archiwum nietknięte).
- ✅ Poprawka przy okazji: Ctrl+L ze ścieżką *do wnętrza* archiwum (także ZIP) dawało „nie można odnaleźć
  ścieżki” — `resolve_path` sprawdzał tylko dysk.
- ✅ Archiwa tylko do odczytu (7z, RAR, tar, gz) blokują od razu, bez okna potwierdzenia: F8, F7, Shift+F4, F2,
  Ctrl+M, Shift+F5, Ctrl+V, F6 z archiwum, przeciąganie na archiwum (`inReadOnlyArchive` + `needWritable`
  w App.svelte). F6 zwykłego pliku `x.tar.gz` z dysku nie jest blokowane (sprawdzany katalog, nie plik).
  Cel wpisany ręcznie w oknie F5/F6 nadal odrzuca backend (frontend nie odróżni pliku `x.tar.gz` od archiwum).
  Niesprawdzone w oknie (typy + test rozpoznawania).

## Kolejka zadań (sesja 2026-11-13, rano) — decyzja użytkownika: stabilność > szybkość
- ✅ Kopiowanie / przenoszenie / pakowanie (F5, F6, Alt+F5, Ctrl+V, synchronizacja) idą **po kolei**: jedno
  połączenie FTP naraz, przewidywalna kolejność, brak wyścigu dwóch zadań o te same pliki. Kolejka jest
  w UI (`jobs.svelte.ts`), backend bez zmian — zadanie dostaje id z `start_job` dopiero, gdy ruszy.
- Poza kolejką: kosz / usuwanie (szybkie), pobieranie do podglądu F3/F4/Enter (`keepPanels` — ktoś czeka).
- Karta czekającego zadania: „Uruchom teraz” (rusza obok trwającego) i „Anuluj”. Nad kartami pasek
  „Kolejka · czeka: N” z „Wstrzymaj / Wznów kolejkę” (trwające zadanie leci dalej, następne nie startują).
- Zadanie z błędem nie blokuje kolejki — karta z błędami zostaje, rusza następne. Odmowa startu
  (`start_job` zwraca błąd) w kolejce = karta z błędem; przy pustej kolejce — błąd w oknie dialogowym jak dotąd.
- Po zakończeniu zadania odznaczane są tylko jego pliki (wcześniej całe zaznaczenie w obu panelach —
  przy kolejce zjadałoby to, co użytkownik zaznacza do następnego zadania).
- Test logiki (mock backendu, 9 scenariuszy): `node scripts/test-queue.mjs`.
- ✅ **Przeklikane w oknie** (3 × 16 MB na lokalny FTP 2 MB/s): wysyłki po kolei (log serwera: 16→24→30→36 s),
  pauza trzyma kolejkę, „Start now” przy pauzie, „Cancel” czekającego (plik nie trafił na serwer), wznowienie,
  kopia identyczna (hash). Program zamyka się czysto.
- Testy UI: hasło do `local-test` jest zapamiętane — po Ctrl+F → Enter **nie** ma okna hasła, kolejny Enter
  trafia w plik. Zatwierdzać okna kliknięciem w przycisk, nie Enterem. Sterownik `ui.ps1` ma `click x,y`
  (współrzędne ze zrzutu), ale każde wywołanie trwa ~2 s, a karty zadań są przyklejone do dołu — przed
  klikaniem w karty wstrzymać kolejkę, żeby układ się nie przesuwał.

## Przegląd bezpieczeństwa danych (sesja 2026-11-13) — priorytet użytkownika: pliki nie mogą ulec uszkodzeniu
- ✅ FTP upload: ponowienie dokleja (APPE) tylko po tym, jak STOR w tym zadaniu dotarł do serwera (wcześniej
  nieudane nadpisanie mogło skleić końcówkę nowego pliku ze starą treścią). Przerwana/pominięta wysyłka
  **nowego** pliku kasuje niepełną kopię. Nadpisywanie dalej wprost (bez `.part` + RNTO) — RNTO zmienia uprawnienia.
- ✅ FTP download: do `<nazwa>.part`, podmiana po weryfikacji; wznowienie obcina plik do offsetu; więcej bajtów
  niż w listingu (link, rosnący log) ≠ zerwane połączenie. Nazwy z `:*?"<>|` → `_`.
- ✅ Dowiązania na FTP: F8 usuwa sam link (DELE), kopiowanie / Spacja / porównanie nie wchodzą w linki do
  katalogów; MLSD `type=OS.unix=slink|symlink` rozpoznawane (`mlsd_unix_link`).
- ✅ Przenoszenie między serwerami nie kasuje źródeł pominiętych plików; nieczytelny katalog docelowy = pominięcie
  (nie nadpisanie bez pytania); konflikty bez rozróżniania wielkości liter; F2/Ctrl+M na FTP nie nadpisuje.
- ✅ F4: kopie w `%TEMP%\twinyard\~edit` (osobno od F3), ponowne F4 = ta sama kopia (niewysłane zmiany zostają),
  zapis zgłaszany, gdy plik przestał się zmieniać, jedna wysyłka naraz na plik; sprzątanie `~edit` po 30 dniach.
  Kopie z archiwów mają w ścieżce hash ścieżki archiwum.
- ✅ Synchronizacja: serwer bez MLSD (daty LIST) → brak domyślnego kierunku dla „nowszy” + ostrzeżenie;
  błąd tworzenia podkatalogu pomija grupę (pojedynczy plik nie trafi pod nazwę katalogu).
- ✅ Ścieżki > 260 znaków w CopyFileExW/MoveFileExW (`\\?\`), nieczytelne wpisy przy skanowaniu raportowane,
  kopiowanie kilku elementów do nowego katalogu go tworzy, nieczytelny `passwords.json` = błąd (nie nadpisanie).
- Przetestowane przez użytkownika na serwerze produkcyjnym (F4, wznawianie). Testy live na pyftpdlib nie były uruchamiane
  (skrypt serwera był w scratchpadzie starej sesji) — do odtworzenia.

## Przegląd nr 3 (sesja 2026-11-13, po południu) — lista w `review-3-pl.md`
- ✅ A1: **przenoszenie (F6) między dwoma serwerami jest wyłączone** (decyzja użytkownika — i tak tak nie
  pracuje): komunikat „skopiuj F5, sprawdź, usuń F8”. Kopiowanie serwer↔serwer zostaje, ale nazwy ze znakami
  niedozwolonymi w Windows dają błąd (nie są już po cichu gubione / zmieniane na `_`). Pobieranie sprawdza
  wolne miejsce na dysku (C12, dotyczy też `%TEMP%` przy kopiowaniu serwer↔serwer).
- ✅ A2: ZIP — jedna zmiana archiwum naraz (blokada per plik w `archive/mod.rs`; zadania czekają, F2/F7
  odmawiają „archiwum jest zmieniane”); plik tymczasowy `<nazwa>.zip.<pid>.part`.
- ✅ A3: ZIP „Nadpisz” z nieczytelnym (zablokowanym) plikiem zostawia starą wersję + błąd.
- ✅ A4: FTP upload — rozmiar z otwartego pliku, wysyłane dokładnie tyle bajtów; „na serwerze więcej niż
  plik” = STOR od zera, nigdy APPE. Plik otwierany bez zgody na zapis przez innych (log otwarty do zapisu →
  otwierany współdzielony). Bez `SIZE` weryfikacja z listingu (C4).
- ✅ A5: Shift+F4 na FTP sprawdza serwer sam (świeży listing w silniku); istniejący plik nie jest zerowany,
  tylko otwierany do edycji.
- ✅ A6: pytania z zadań (konflikt, błąd) ignorują klawisze/kliki przez 600 ms i zawsze przytrzymany klawisz.
- ✅ B1 (częściowo): przed uznaniem pliku `.*` za nowy — `SIZE` (serwery ukrywające `.htaccess` w LIST).
  Sprawdzone przez użytkownika (2026-11-13): dwa serwery produkcyjne (IIS, `web.config`) pokazują pliki
  `.*` w panelu — pobieranie/synchronizacja ich nie gubią; `LIST -a` niepotrzebne.
- ✅ B2 `mkdir_p`, B3 nieudane łączenie liczone jak zerwanie, 5 prób z odstępami 1/3/6/15/30 s.
- ✅ B4: zamknięcie programu z zadaniami — najpierw anulowanie i czekanie (do 15 s), potem zamknięcie.
- ✅ B5: nadpisywanie lokalne przez `<nazwa>.twinyard-part` + podmiana (też C10: ukryte/systemowe cele).
  Rozpakowanie z archiwum na istniejący plik — nadal wprost (do zrobienia przy okazji).
- ✅ B6 pakowanie zgłasza pominięte/nieczytelne, B7 za dowiązanie uznawane tylko symlink/junction
  (`win::is_link`; OneDrive „na żądanie” to zwykłe pliki), B8 Ctrl+M: konflikt z plikami spoza zaznaczenia
  w podglądzie + powrót do starej nazwy po błędzie, B9 `panic = "unwind"` + `catch_unwind` w wątku zadania.
- ✅ C1 (F4: dwie nazwy → jedna kopia = błąd), C3 (`fetch_to` z kontrolą rozmiaru), C5 (plik krótszy niż w
  listingu kończy się; serwer bez `REST` → od zera), C6 (`..` w nazwie w ZIP, kolizja bez wielkości liter).
- Nie ruszane (rzadkie): C2 (lista F4 tylko w pamięci), C7 (sync, nazwy różniące się wielkością liter),
  C8 (zwykły katalog `x.zip\`), C9 (kodowanie nazw per sesja), C11 (`\` w nazwie zdalnej).
- WebView2 bez pobierania dodatków Edge (`--disable-component-update` w `tauri.conf.json`; Widevine DRM, filtry
  reklam, mowa — ~60 MB w `data\webview`, twinyard ich nie używa). Własne `additionalBrowserArgs` zastępują
  domyślne wry, więc jego `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection` jest przepisane.
- Tryb ciemny: neutralne szarości jak VS Code (Dark Modern), niebieski tylko jako stonowany akcent.
- Testy: core 41 ✅ (nowe: zablokowany plik przy nadpisaniu w ZIP, 6 równoległych dodań do ZIP, złe nazwy
  w ZIP, nadpisanie ukrytego pliku). Live FTP 11/11 ✅ na lokalnym pyftpdlib (serwery z poprzedniej sesji
  wciąż działały; zapasowy skrypt 2121/2123/2126 w scratchpadzie tej sesji).

**NASTĘPNA SESJA — propozycje:** Alt+F5 do 7z (sevenz-rust2 umie pakować); wyszukiwanie Alt+F7
wewnątrz archiwów; .tar/.gz/.tgz. (Tworzenie archiwów z hasłem — na razie niepotrzebne.)

## Plan (MVP → dalej)
1. ✅ **Prototyp UI-ryzyka:** okno Tauri, panele, lista 100k elementów, nawigacja klawiaturą, motyw ciemny/jasny.
2. ✅ Dwa panele, taby, lokalny FS, F5/F6/F7/F8, Shift+F6/F2 rename, zaznaczanie.
3. ✅ Kolejka operacji z postępem, konfliktami, kosz.
4. ✅ Konfiguracja: edytor F4, podgląd F3, porównywarka Ctrl+F2; ulubione (Ctrl+D, też ręczne
   dodawanie/edycja np. udziałów UNC); Shift+F4 nowy plik + edytor; edytowalna ścieżka (klik / Ctrl+L:
   %ZMIENNE%, względne, plik → katalog z kursorem na pliku); po F7/F2 odświeża się drugi panel w tym
   samym katalogu. Brakuje: edycja skrótów, Shift+F5 (kopia w tym samym katalogu), import hotlisty
   z TC. ✅ Auto-odświeżanie przy zmianach z zewnątrz (`watch.rs`, crate `notify` =
   ReadDirectoryChangesW, tylko katalogi widoczne w panelach, bez rekursji, zdarzenia scalane co 300 ms).
5. ✅ FTP/FTPS (bez SFTP — użytkownik go nie używa) + menedżer połączeń (Ctrl+F / przycisk FTP
   na pasku dysków, Ctrl+Shift+F rozłącz). Szczegóły niżej w „Stan kroku 5”.
6. ✅ Import z TC: połączenia + przypięte certyfikaty (bez haseł). Hotlista `[DirMenu]` — czeka na
   `wincmd.ini`.
7. Szlify: szybki filtr/wyszukiwanie, porównanie katalogów (synchronizacja), archiwa (zip/7z), podgląd F3.

## Ustalenia z użytkownikiem (2026-11-12)
- Stos: decyzja po stronie Claude → **Tauri 2 + Svelte 5 + Rust** (jak wyżej).
- Platforma: **tylko Windows** — można śmiało używać Win32 API, bez abstrakcji wieloplatformowych.
- Protokoły wg priorytetu: **FTPS (implicit 990 + explicit)** i FTP → SFTP później
  (użytkownik obecnie nie ma połączeń SFTP).
