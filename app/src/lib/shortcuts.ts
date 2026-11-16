import { t } from "./i18n.svelte";

/**
 * All keyboard shortcuts, for the help window (F1). Keep in sync with
 * App.svelte onKey. A function, so the texts follow the interface language.
 */
export function shortcuts(): { group: string; items: [string, string][] }[] {
  return [
    {
      group: t("Nawigacja"),
      items: [
        ["Tab", t("Przełącz panel")],
        [t("Enter / dwuklik"), t("Wejdź do katalogu / otwórz plik")],
        ["Backspace, Ctrl+PgUp", t("Katalog wyżej")],
        ["Ctrl+PgDn", t("Wejdź do katalogu")],
        ["↑ ↓ PgUp PgDn Home End", t("Ruch kursora")],
        [t("litery"), t("Szybkie szukanie po nazwie (ta sama litera = następny)")],
        [t("Ctrl+L / klik w ścieżkę"), t("Wpisz ścieżkę (też %ZMIENNE%, plik, ftp://…)")],
        ["Alt+← / Alt+→", t("Wstecz / dalej w historii katalogów (też boczne przyciski myszy)")],
        ["Alt+↓", t("Lista ostatnio odwiedzonych katalogów")],
        ["Ctrl+S", t("Szybki filtr (*.php;*.js albo fragment nazwy), Esc czyści")],
        ["Ctrl+R", t("Odśwież (zmiany z zewnątrz odświeżają się same)")],
        ["Ctrl+H", t("Pokaż / ukryj pliki ukryte i systemowe")],
        ["Ctrl+U", t("Zamień panele miejscami")],
        ["Ctrl+← / Ctrl+→", t("Katalog spod kursora (albo bieżący) w lewym / prawym panelu")],
        ["Ctrl+F3 / F4 / F5 / F6", t("Sortuj: nazwa / rozszerzenie / data / rozmiar")],
        ["Ctrl+F1", t("Widok skrócony (same nazwy w kolumnach) / pełny; w skróconym ← → skacze o kolumnę")],
      ],
    },
    {
      group: t("Zaznaczanie"),
      items: [
        ["Insert", t("Zaznacz / odznacz")],
        [t("Spacja"), t("Zaznacz; na katalogu też policz jego rozmiar (Esc przerywa)")],
        ["Alt+Shift+Enter", t("Policz rozmiary wszystkich katalogów")],
        ["Ctrl+Q", t("Rozmiar oraz liczba plików i katalogów zaznaczonych (albo spod kursora)")],
        ["Shift+↑ / Shift+↓", t("Zaznaczaj idąc")],
        ["Shift+PgUp / Shift+PgDn", t("Zaznacz stronę w górę / w dół")],
        ["Shift+Home / Shift+End", t("Zaznacz wszystko do początku / do końca")],
        [t("Ctrl+klik"), t("Zaznacz myszą")],
        [t("Przeciągnij prawym"), t("Zaznacz (albo odznacz) przeciągnięte pliki; sam prawy klik — menu kontekstowe")],
        ["Ctrl+A", t("Zaznacz wszystko")],
        ["Num + / Num −", t("Zaznacz / odznacz według maski (*.php;*.js, „*\\” = katalogi)")],
        ["Ctrl+Num −", t("Odznacz wszystko")],
        ["Num *", t("Odwróć zaznaczenie")],
      ],
    },
    {
      group: t("Pliki"),
      items: [
        ["F3", t("Podgląd (tekst, hex, obrazy)")],
        ["F4", t("Edycja (pliki z FTP: po zapisie pytanie o wysłanie)")],
        ["Shift+F4", t("Nowy plik tekstowy")],
        ["F5", t("Kopiuj do drugiego panelu")],
        ["Shift+F5", t("Kopiuj w tym samym katalogu (pod nową nazwą)")],
        ["Alt+F5", t("Spakuj zaznaczone do ZIP (w drugim panelu)")],
        [t("Enter na .zip / .7z / .rar / .tar / .tgz / .gz"), t("Wejdź do archiwum jak do katalogu; F5 z archiwum = rozpakuj")],
        [t("W ZIP: F5/F6/przeciąganie, F8, F2, F7"), t("Dopisz, usuń, zmień nazwę, nowy katalog (pozostałe formaty tylko do odczytu)")],
        ["F6", t("Przenieś do drugiego panelu")],
        ["F2, Shift+F6", t("Zmień nazwę")],
        ["Ctrl+M", t("Zmiana nazw wielu plików (maska, licznik, znajdź/zamień, podgląd)")],
        ["F7", t("Nowy katalog (można a\\b\\c)")],
        ["F8, Delete", t("Do kosza (na FTP i w archiwum: usuń)")],
        ["Shift+F8, Shift+Delete", t("Usuń trwale")],
        ["Ctrl+F2", t("Porównaj pliki (2 zaznaczone albo pod kursorami w obu panelach)")],
        ["Alt+F7", t("Szukaj plików (nazwa, tekst)")],
        [t("Prawy klik, klawisz Menu, Shift+F10"), t("Menu kontekstowe Eksploratora (7-Zip, Wyślij do, Właściwości…)")],
        [t("Przeciąganie myszą"), t("Na drugi panel / katalog = kopiuj (Shift = przenieś); poza okno = do Eksploratora")],
        ["Shift+F2", t("Porównaj / synchronizuj katalogi obu paneli (też dysk ↔ FTP)")],
        ["Ctrl+C / Ctrl+X / Ctrl+V", t("Schowek plików — wspólny z Eksploratorem (wklejanie też na FTP i do ZIP)")],
        ["Ctrl+Shift+C / Ctrl+Alt+C", t("Pełne ścieżki / same nazwy do schowka jako tekst")],
      ],
    },
    {
      group: t("Zakładki"),
      items: [
        ["Ctrl+T", t("Nowa zakładka")],
        [t("Ctrl+W, środkowy klik"), t("Zamknij zakładkę")],
        ["Ctrl+Tab / Ctrl+Shift+Tab", t("Następna / poprzednia zakładka")],
        ["Ctrl+↑", t("Otwórz katalog spod kursora w nowej zakładce")],
      ],
    },
    {
      group: t("Ulubione, FTP, inne"),
      items: [
        ["Ctrl+D", t("Ulubione katalogi (A dodaj bieżący, R dodaj ręcznie)")],
        ["Ctrl+F", t("Połączenia FTP")],
        ["Ctrl+Shift+F", t("Rozłącz FTP w aktywnym panelu")],
        ["F1", t("Ta lista")],
      ],
    },
    {
      group: t("Podgląd (F3) i porównanie (Ctrl+F2)"),
      items: [
        ["Ctrl+F, F7, /", t("Szukaj w podglądzie; N następny")],
        ["W / H", t("Zawijanie wierszy / widok hex")],
        ["N, F7, Alt+↓ / P, Alt+↑", t("Następna / poprzednia różnica")],
        [t("→ / ← / spacja, S"), t("Synchronizacja: kierunek kopiowania / pomiń; S uruchamia")],
        ["Esc", t("Zamknij")],
      ],
    },
  ];
}
