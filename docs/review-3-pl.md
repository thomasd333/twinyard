# Przegląd kodu nr 3 (2026-11-13) — lista do naprawy

> **Stan (2026-11-13, po południu):** naprawione A1–A6, B2–B9, C1, C3–C6, C10, C12; B1 częściowo
> (SIZE dla plików `.*` przy wysyłaniu). A1 rozwiązane przez **wyłączenie przenoszenia między serwerami**.
> Pominięte (rzadkie): C2, C7, C8, C9, C11. Szczegóły: README, sekcja „Przegląd nr 3”.


Tylko namierzone, nic nie zmieniono w kodzie. Przegląd przez czytanie kodu (Rust: `crates/core`,
`crates/remote`, `app/src-tauri`; frontend: `App.svelte`, kolejka, synchronizacja, Ctrl+M).
Testów ani builda nie uruchamiałem. Punkty oznaczone „do sprawdzenia” to hipotezy — reszta wynika
wprost z kodu.

Kolejność = proponowana kolejność naprawy.

## A. Poważne — realna utrata lub uszkodzenie danych

### A1. FTP serwer↔serwer: plik z „niewindowsową” nazwą znika przy przenoszeniu
`crates/remote/src/engine.rs:853-886` (`relay`), `:79` (`win_name`), `:413-415`.
Pobieranie do katalogu tymczasowego zamienia `: * ? " < > | \` na `_`, a `relay` szuka potem pliku
pod **oryginalną** nazwą (`tmp.join(file_name(source))`) i `.filter(|p| p.exists())` po cichu go
pomija. Plik nie jest wysyłany, błędu nie ma, więc przy F6 warunek „bez błędów i pominięć” jest
spełniony i źródło zostaje **usunięte**. Przy F5 — plik po cichu nieskopiowany, zadanie „udane”.
Przykład: `backup_2026-11-13_10:30.sql` na serwerze linuksowym.
Pliki głębiej w katalogach trafiają na serwer docelowy z `_` w nazwie (cicha zmiana nazwy).
Kierunek: lista lokalnych źródeł z faktycznie utworzonych nazw (albo mapa oryginał → nazwa lokalna
i wysyłka pod oryginalną nazwą); brak pliku w tmp = błąd, nigdy cisza.

### A2. ZIP: dwie zmiany tego samego archiwum naraz
`crates/core/src/archive/zipfmt.rs:108-124` (`rewrite`), `:99` (`part_path`).
Każda zmiana czyta oryginał i pisze do tego samego `<nazwa>.zip.part`; nie ma blokady na archiwum.
Dwie równoległe operacje: (a) wygrywa ostatnia — zmiana pierwszej przepada; (b) obie piszą do
jednego `.part` (drugie `File::create` obcina plik pierwszej) → podmienione archiwum to śmieci.
Przy F6 **do** archiwum źródła są już usunięte (`mod.rs:806-819`), a późniejsza podmiana z drugiej
operacji ich nie zawiera.
Jak do tego dojść: kolejka pilnuje tylko copy/move/pack (`app/src/lib/jobs.svelte.ts:36`), a poza nią
idą: F8 w archiwum, F2 / Ctrl+M / F7 (`lib.rs:108`, `:255` — zwykłe komendy), aktualizacja po F4
(`edits.rs:155`), „Uruchom teraz”.
Kierunek: globalna mapa blokad per ścieżka archiwum (mutex trzymany na całe `rewrite`, wpisy czytane
już pod blokadą) + unikalna nazwa pliku tymczasowego.

### A3. ZIP: „Nadpisz” z nieczytelnym źródłem kasuje stary wpis
`crates/core/src/archive/mod.rs:789-792` + `zipfmt.rs:130`, `:167-174`.
Nadpisanie = stary wpis w `plan.drop` + nowy w `plan.add`. Jeśli nowego pliku nie da się otworzyć
(zablokowany przez inny program), `write_part` robi `continue` — stary wpis już pominięto, nowego
nie ma. Błąd jest pokazany, ale poprzednia wersja znika z archiwum.
Kierunek: otwierać pliki źródłowe przed przepisaniem starych wpisów albo przy niepowodzeniu
przerwać całe `rewrite` (oryginał zostaje).

### A4. FTP upload: plik, który zmienił rozmiar, dostaje doklejoną końcówkę
`crates/remote/src/engine.rs:698-717` (`upload_file`), `:751-785` (`put_file`).
`size` pochodzi ze skanu. Jeśli plik urósł (zapis w edytorze, rosnący log), `put_file` wysyła
całość, `SIZE` ≠ `size` → `E::Incomplete` → traktowane jak zerwane połączenie → ponowienie z
`off = min(SIZE, size) = size` → **APPE** bajtów `[size..EOF]` na pliku, który już je ma. Trzy
ponowienia = końcówka doklejona do 3 razy, potem błąd. Jeśli plik zmalał — trzy puste ponowienia
i błąd, choć plik na serwerze jest dobry.
Najbardziej realne przy F4 z „wysyłaj automatycznie” i autozapisem edytora (zapis w trakcie wysyłki).
Kierunek: oczekiwany rozmiar brać z uchwytu otwartego pliku (i liczyć faktycznie wysłane bajty);
plik otwierać bez zgody na zapis przez innych; „na serwerze więcej niż wysłano” nie może prowadzić
do APPE — tylko STOR od zera albo błąd.

### A5. Shift+F4 na FTP potrafi wyzerować istniejący plik
`app/src/App.svelte:741-742`, `app/src-tauri/src/edits.rs:185-195`.
To, czy plik istnieje, sprawdza frontend w **liście panelu** (na FTP nie odświeża się sama).
Jeśli plik powstał po ostatnim listowaniu, idzie `edit_new_remote`: pusty plik wysyłany z hostem
`Silent`, który na konflikt odpowiada „Nadpisz” → istniejący plik na serwerze ma 0 bajtów.
Kierunek: sprawdzenie w backendzie na świeżym listingu; istnieje → ścieżka `edit_remote`.

### A6. Okno konfliktu: Enter = „Nadpisz”, a okno pojawia się samo
`app/src/lib/JobsView.svelte:21` (`primary` na „Nadpisz”), `app/src/lib/Modal.svelte:42-44`.
Zadania lecą w tle, a użytkownik w tym czasie chodzi po panelach Enterem. Pytanie o konflikt może
wyskoczyć dokładnie pod naciśnięty Enter → plik nadpisany bez przeczytania pytania (przy
przytrzymanym klawiszu kilka z rzędu). To samo z „Ponów” w oknie błędu (niegroźne).
Kierunek: ignorować klawisze przez ~500 ms od pojawienia się pytania z zadania i/lub brak przycisku
domyślnego w oknie konfliktu.

## B. Średnie — rzadsze albo „tylko” utrata stabilności

### B1. Serwery ukrywające pliki z kropką (`.htaccess`) — do sprawdzenia na Twoich serwerach
`crates/remote/src/ftp.rs:141-147` (zwykłe `LIST` / `MLSD`), `engine.rs:631-639`, `:669`, `:718-725`.
Część serwerów nie pokazuje plików `.*` bez `LIST -a`. Skutki, jeśli tak jest:
- wysłanie `.htaccess` nie widzi konfliktu → nadpisanie bez pytania; plik uznany za „nowy”
  (`fresh`), więc przy błędzie / anulowaniu wysyłki istniejący `.htaccess` zostaje **usunięty**;
- pobieranie katalogu i synchronizacja pomijają te pliki po cichu.
Test: czy panel pokazuje `.htaccess` na serwerach produkcyjnych. Jeśli tak — punkt nieaktualny.
Kierunek: `LIST -a` z powrotem do `LIST` przy błędzie; przed uznaniem pliku za „nowy” — `SIZE`.

### B2. `mkdir_p` może wpisać do cache pusty listing istniejącego katalogu
`crates/remote/src/engine.rs:607-625`.
`remote_is_dir` po błędzie i „Pomiń” zwraca `false`; potem pętla CWD/MKD przechodzi (katalog
istnieje) i `dir_cache.insert(dir, Vec::new())`. Od tej chwili każdy plik w tym katalogu jest
„nowy”: nadpisanie bez pytania, a przy błędzie usunięcie istniejącego pliku (jak w B1).
Kierunek: pusty wpis tylko wtedy, gdy MKD faktycznie utworzyło ostatni segment.

### B3. Nieudane ponowne łączenie nie jest ponawiane automatycznie
`crates/remote/src/engine.rs:173-178`, `:40-46`, `:195-206`.
`open_session` zwraca `E::Msg`, a `reconnect()` jest prawdziwe tylko dla `E::Ftp` / `E::Incomplete`.
Czyli: zerwanie → czekamy 1 s → próba połączenia się nie udaje (sieć leży 10 s) → od razu okno
z pytaniem, bez prób 2 i 3. Długi transfer bez nadzoru staje na oknie dialogowym. Łączny czas
automatycznych prób (1+3+6 s) też jest krótki jak na restart routera.
Kierunek: błąd łączenia w trakcie ponawiania liczyć jak błąd połączenia; dłuższy backoff.

### B4. Zamknięcie programu w trakcie zadania ucina je w połowie
`app/src/App.svelte:221-236`, `app/src/lib/uiState.ts` (`onCloseRequested`).
Po potwierdzeniu okno się zamyka, proces ginie razem z wątkami. Lokalna kopia (CopyFileExW) zostawia
niepełny plik pod **docelową** nazwą, wysyłka nadpisująca — ucięty plik na serwerze. Taki plik ma
świeżą datę, więc synchronizacja uzna go za „nowszy” i zaproponuje skopiowanie na dobrą wersję.
Kierunek: po potwierdzeniu anulować wszystkie zadania, poczekać na `job-done` (z limitem czasu),
dopiero wtedy zamknąć.

### B5. Nadpisywanie „w miejscu” — przerwanie niszczy starą wersję
- lokalnie: `crates/core/src/ops.rs:425-431` — `CopyFileExW` na istniejący plik; anulowanie lub
  błąd w połowie = Windows kasuje cel, stara wersja przepada;
- rozpakowanie z archiwum: `crates/core/src/archive/mod.rs:484-491` (`File::create` na cel),
  `:534-541` (`abort` usuwa plik).
Pobieranie z FTP ma już `.part` + podmianę; tu tego brakuje. Kierunek: ten sam wzorzec.
(Wysyłka na FTP nadpisuje wprost świadomie — decyzja z poprzedniego przeglądu.)

### B6. Pakowanie / dodawanie do ZIP po cichu pomija pliki
`crates/core/src/archive/mod.rs:666-684` (`walk`).
Nieczytelny katalog (`if let Ok(rd)`), nieczytelny wpis (`rd.flatten()`), błąd `symlink_metadata`
i **każdy** obiekt z atrybutem reparse (także pliki) — bez żadnego komunikatu. Archiwum wygląda na
kompletne. `ops.rs` i `engine.rs` zgłaszają to od poprzedniego przeglądu; tu zostało po staremu.

### B7. Foldery chmurowe (OneDrive itp.) mogą być brane za dowiązania — do sprawdzenia
Wszędzie `attrs & ATTR_REPARSE` bez patrzenia na tag: `ops.rs:253`, `:268`, `:377-383`;
`engine.rs:364`; `dircmp.rs:74`; `extras.rs:86`; `archive/mod.rs:668`.
Katalogi OneDrive z „Pliki na żądanie” bywają punktami reparse (tag `IO_REPARSE_TAG_CLOUD_*`), nie
dowiązaniami. Wtedy: F5 katalogu = „Pominięto dowiązanie”, porównanie katalogów go nie widzi,
Spacja liczy 0, pakowanie pomija po cichu. Na tym komputerze OneDrive nie ma takich folderów, więc
nie potwierdziłem. Kierunek: za link uznawać tylko tagi „name surrogate” (junction, symlink).

### B8. Ctrl+M: konflikt z plikiem spoza zaznaczenia, nazwy tymczasowe zostają
`app/src/lib/MultiRename.svelte:79-89` (podgląd), `:99-126`.
Podgląd wykrywa powtórzenia tylko wśród zmienianych nazw, nie względem reszty katalogu. W trybie
dwufazowym plik, którego docelowa nazwa jest zajęta, zostaje z nazwą `x.~fcmd…` (jest błąd, ale
nie ma cofnięcia). Kierunek: przekazać pełną listę nazw katalogu do podglądu; przy błędzie fazy 2
wrócić do starej nazwy.

### B9. `panic = "abort"` w wydaniu
`Cargo.toml:22`, `app/src-tauri/src/jobs.rs:116-135`.
Każdy panic w dowolnym wątku zabija cały program w połowie operacji (skutki jak w B4). Konkretnego
miejsca, które panikuje, nie znalazłem — to uwaga o braku siatki bezpieczeństwa. Kierunek:
`panic = "unwind"` + `catch_unwind` w wątku zadania, który zawsze wyśle `job-done` z błędem.

## C. Drobne

- **C1. F4: dwie nazwy zdalne → jedna kopia lokalna.** `manager.rs:290-301`, `edits.rs:114-119`.
  `Index.php` i `index.php` (albo `a:b` i `a_b`) w jednym katalogu dają ten sam plik w `%TEMP%`,
  ale dwa wpisy obserwatora → zapis wysyła treść do **obu** plików na serwerze.
- **C2. F4 po restarcie nadpisuje niewysłaną kopię.** `edits.rs:88-107`. Lista obserwowanych plików
  żyje tylko w pamięci; ponowne F4 pobiera plik na kopię z niewysłanymi zmianami. Przy zamykaniu
  programu nie ma ostrzeżenia o niewysłanych zmianach.
- **C3. `fetch_to` bez kontroli rozmiaru.** `manager.rs:252-279`. Małe pliki (F3/F4 < 1 MB) nie są
  porównywane z rozmiarem z listingu; ucięte pobranie + zapis w edytorze = ucięty plik na serwerze.
- **C4. Przenoszenie na serwer bez `SIZE`.** `engine.rs:709-713`, `:738-744`. Gdy serwer nie zna
  `SIZE`, weryfikacja jest pomijana, a plik lokalny i tak usuwany. Zapas: rozmiar z listingu.
- **C5. Pobieranie pliku krótszego niż w listingu nigdy się nie kończy** (`engine.rs:475-483`):
  ponowienia, potem błąd, „Ponów” w kółko. Brak też powrotu do pobierania od zera, gdy serwer
  odrzuca `REST` (`:529-531`).
- **C6. ZIP: `..` w nazwie docelowej.** `archive/mod.rs:734-746` (`add`), `:911-926` (`rename`) nie
  używają `norm_name` (robi to tylko `mkdir`). Wpis `../x` trafia do archiwum, a twinyard go potem
  nie pokazuje; przy F6 źródło jest już usunięte. W `rename` sprawdzenie „już istnieje” rozróżnia
  wielkość liter (`:921`), w `add` nie.
- **C7. Synchronizacja z FTP przy nazwach różniących się wielkością liter.** `dircmp.rs:122-129`,
  `SyncView.svelte:162`. Ścieżka źródła budowana z nazwy lewej strony: `←` daje „Nie znaleziono na
  serwerze”, `→` tworzy na Linuksie drugi plik obok.
- **C8. Zwykły katalog nazwany jak archiwum** (`dane.zip\`, `x.tar\`). `api.ts:192-201` rozpoznaje
  po samej nazwie: F8 usuwa trwale zamiast do kosza, a w katalogu `*.7z` / `*.tar` wszystkie
  operacje zapisu są zablokowane.
- **C9. Wykrywanie kodowania nazw jest per sesja.** `names.rs:25-52`. Zadanie otwiera nową sesję
  z „czystym” stanem i wysyła pierwsze ścieżki w CP1250, nawet gdy sesja panelu wykryła UTF-8 →
  „nie znaleziono” albo katalog o zepsutej nazwie. Dotyczy tylko trybu „auto” bez UTF8 w FEAT.
- **C10. Nadpisanie lokalnego pliku ukrytego / systemowego kończy się „Odmowa dostępu”.**
  `ops.rs:402-405` zdejmuje tylko „tylko do odczytu”, a `CopyFileExW` odmawia też dla ukrytych.
- **C11. Zdalna nazwa z `\`** (dozwolona na Linuksie) jest traktowana jak separator
  (`remote/src/path.rs:14`).
- **C12. Kopiowanie serwer↔serwer idzie przez `%TEMP%`** bez sprawdzenia wolnego miejsca
  (`engine.rs:854-864`) — duże transfery zapełnią dysk systemowy.

## Sprawdzone, bez zastrzeżeń
Przenoszenie między woluminami (źródło usuwane po udanej kopii), pobieranie z FTP (`.part`, obcięcie
przy wznowieniu, podmiana), podmiana archiwum ZIP przy pojedynczej operacji, usuwanie na FTP
(dowiązania = DELE), sprzątanie `%TEMP%` (nie wchodzi w linki, tylko stare pliki), zapis
`connections.json` / `passwords.json` / `state.json` (atomowy), maskowanie hasła w logu, weryfikacja
certyfikatów w obu trybach TLS, porządek sortowania naturalnego (spójny — sort nie spanikuje),
niedomknięty strumień danych (suppaftp sam kończy transfer w `Drop`).
