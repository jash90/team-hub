/**
 * Polish for what the server says. The server writes English (error messages, notification
 * texts, the status lines of agent runs, import reports); `translateServer` in `server.ts`
 * puts it into Polish on arrival, with these two tables. The keys are the server's texts,
 * letter for letter; `npm run build` checks them against the Rust sources both ways
 * (`scripts/check-server-messages.mjs`), so a new message without Polish, or an entry whose
 * English no longer exists, stops the build.
 *
 * `exact` holds whole texts, and the short phrases the server puts into longer ones
 * ("Project" in "Project not found"). Those phrases are written the way they read after a
 * colon or in brackets, in the nominative, because that is where the patterns put them.
 *
 * `patterns` holds texts with variable parts. In a key, `{name}` stands for a variable part;
 * the name is free, except that `{key}` stands for an issue key such as WEB-12 and matches
 * nothing else. In the Polish, `{name}` puts the part back as it came (names, titles, what
 * people wrote), and `{t:name}` translates it first (for a part that is itself one of the
 * server's phrases or messages). Polish is phrased so that a part never needs declension:
 * "Nie znaleziono: projekt", not "Nie znaleziono projektu".
 *
 * This file imports nothing: the build's check loads it as it is.
 */
export const exact: Record<string, string> = {
  // ---- modules, as Settings lists them ----
  'AI agents (MCP)': 'Agenci AI (MCP)',
  'Lets coding agents such as Claude Code read and update issues through the Model Context Protocol.':
    'Pozwala agentom programistycznym, takim jak Claude Code, czytać i zmieniać zadania przez Model Context Protocol.',
  'Confluence import': 'Import z Confluence',
  'Imports Confluence spaces into the knowledge base and keeps the copy up to date.': 'Importuje przestrzenie Confluence do bazy wiedzy i utrzymuje kopię na bieżąco.',
  'Tests (e2e)': 'Testy (e2e)',
  'Runs of the e2e testing framework, each test linked to the issues it verifies: results on issues, cards and a project page.':
    'Przebiegi frameworka testów e2e, w których każdy test jest powiązany z zadaniami, które sprawdza: wyniki przy zadaniach, na kartach i na stronie projektu.',
  GitHub: 'GitHub',
  'Pull requests, branches and commits on issues; workflow rules driven by code; drift between status and code.':
    'Pull requesty, gałęzie i commity przy zadaniach; reguły workflow sterowane kodem; rozbieżności między statusem a kodem.',
  'Jira import': 'Import z Jiry',
  'Brings projects over from Jira with their issue keys, history, comments, sprints and versions.': 'Przenosi projekty z Jiry razem z kluczami zadań, historią, komentarzami, sprintami i wersjami.',
  Reports: 'Raporty',
  'Burndown, velocity, cumulative flow, cycle time and workload, computed from the issue history.':
    'Wykres spalania, prędkość, przepływ skumulowany, czas realizacji i obciążenie, liczone z historii zadań.',
  Timeline: 'Oś czasu',
  "A project's epics as bars on a time axis, from start date to due date, with sprints and releases behind them.":
    'Epiki projektu jako paski na osi czasu, od daty rozpoczęcia do terminu, na tle sprintów i wydań.',
  'Knowledge base': 'Baza wiedzy',
  'Spaces of Markdown pages with history and search, linked to the issues they describe.': 'Przestrzenie stron w Markdownie, z historią i wyszukiwaniem, powiązane z zadaniami, które opisują.',

  // ---- things by name, as they appear in "{what} not found" and similar ----
  Project: 'projekt',
  project: 'projekt',
  Module: 'moduł',
  Issue: 'zadanie',
  Attachment: 'załącznik',
  Comment: 'komentarz',
  Link: 'powiązanie',
  'Work log': 'wpis czasu pracy',
  Filter: 'filtr',
  Status: 'status',
  status: 'status',
  'A status': 'status',
  'Issue type': 'typ zadania',
  'issue type': 'typ zadania',
  'An issue type': 'typ zadania',
  Component: 'komponent',
  'A component': 'komponent',
  Sprint: 'sprint',
  sprint: 'sprint',
  User: 'użytkownik',
  Token: 'token',
  Version: 'wersja',
  version: 'wersja',
  Someone: 'Ktoś',

  // ---- access and accounts ----
  'Sign in to continue': 'Zaloguj się, aby kontynuować',
  'Wrong e-mail or password': 'Nieprawidłowy e-mail lub hasło',
  'Too many attempts; try again in a few minutes': 'Zbyt wiele prób; spróbuj ponownie za kilka minut',
  'Missing or stale CSRF token; reload the page': 'Brak tokenu CSRF albo token jest nieaktualny; odśwież stronę',
  'This instance is already set up': 'Ta instancja jest już skonfigurowana',
  'You can only view this project': 'Ten projekt możesz tylko przeglądać',
  'Only a project administrator can do that': 'To może zrobić tylko administrator projektu',
  'Only an administrator can do that': 'To może zrobić tylko administrator',
  'Only an administrator can change roles': 'Role może zmieniać tylko administrator',
  'This is the last administrator': 'To ostatni administrator',
  'You can only change your own account': 'Możesz zmieniać tylko własne konto',
  'The password needs at least 8 characters': 'Hasło musi mieć co najmniej 8 znaków',
  'The current password is not right': 'Obecne hasło jest nieprawidłowe',
  'That does not look like an e-mail address': 'To nie wygląda na adres e-mail',
  'A name of 1 to 80 characters is required': 'Podaj imię i nazwisko (od 1 do 80 znaków)',
  'Somebody already uses that e-mail address': 'Ktoś już używa tego adresu e-mail',
  'Give the token a name': 'Nadaj tokenowi nazwę',
  'That would duplicate or orphan existing data': 'Ta zmiana powieliłaby istniejące dane albo zostawiła je bez powiązania',

  // ---- issues ----
  'An issue needs a summary': 'Zadanie musi mieć tytuł',
  'Priority goes from 1 (highest) to 5 (lowest)': 'Priorytet to liczba od 1 (najwyższy) do 5 (najniższy)',
  'Story points go from 0 to 1000': 'Punkty mogą mieć wartość od 0 do 1000',
  'That person does not exist or is no longer active': 'Ta osoba nie istnieje albo jej konto nie jest już aktywne',
  'That issue type does not belong to this project': 'Ten typ zadania nie należy do tego projektu',
  'That status does not belong to this project': 'Ten status nie należy do tego projektu',
  'That sprint does not belong to this project': 'Ten sprint nie należy do tego projektu',
  'That version does not belong to this project': 'Ta wersja nie należy do tego projektu',
  'That component does not belong to this project': 'Ten komponent nie należy do tego projektu',
  'The project has no issue type for this': 'Projekt nie ma odpowiedniego typu zadania',
  'A completed sprint cannot take new issues': 'Do zakończonego sprintu nie można dodawać zadań',
  'A subtask needs a parent issue': 'Podzadanie musi mieć zadanie nadrzędne',
  'The parent must be in the same project': 'Zadanie nadrzędne musi być w tym samym projekcie',
  'An issue cannot be its own parent': 'Zadanie nie może być nadrzędne wobec samego siebie',
  'An epic cannot have a parent': 'Epik nie może mieć zadania nadrzędnego',
  'Only an epic can be the parent of this issue': 'Zadaniem nadrzędnym tego zadania może być tylko epik',
  'A subtask belongs under a story, task or bug': 'Podzadanie musi należeć do historyjki, zadania albo błędu',
  'That parent does not fit: epics hold stories, tasks and bugs, and those hold subtasks':
    'To zadanie nadrzędne nie pasuje: epiki zawierają historyjki, zadania i błędy, a te zawierają podzadania',
  'An issue with subtasks cannot become a subtask': 'Zadanie, które ma podzadania, nie może stać się podzadaniem',
  'An issue number goes from 1 to 9999999': 'Numer zadania to liczba od 1 do 9999999',
  'An issue cannot be linked to itself': 'Zadania nie można powiązać z nim samym',
  'These issues are already linked that way': 'Te zadania są już tak powiązane',
  'The comment is empty': 'Komentarz jest pusty',
  'You can only change your own comments': 'Możesz zmieniać tylko własne komentarze',
  'The file is empty': 'Plik jest pusty',
  'Attachments can be at most 25 MB': 'Załącznik może mieć najwyżej 25 MB',
  'Time spent looks like "1h 30m"': 'Przepracowany czas zapisuje się na przykład tak: „1h 30m”',
  'Time spent looks like "1h 30m", "45m" or "2d"': 'Przepracowany czas zapisuje się na przykład tak: „1h 30m”, „45m” albo „2d”',
  'Time spent must be at least a minute': 'Przepracowany czas to co najmniej minuta',
  'Time spent must be between a minute and a month': 'Przepracowany czas musi mieścić się między minutą a miesiącem',
  'You can only remove your own work logs': 'Możesz usuwać tylko własne wpisy czasu pracy',

  // ---- projects, sprints, versions, filters ----
  'A project key is 2 to 10 letters and digits, starting with a letter': 'Klucz projektu to od 2 do 10 liter i cyfr, zaczyna się od litery',
  'The project needs a name': 'Projekt musi mieć nazwę',
  'A project needs at least one status': 'Projekt musi mieć co najmniej jeden status',
  'Choose a status for the issues that are in this one': 'Wybierz status dla zadań, które mają teraz ten status',
  'Choose a type for the issues that have this one': 'Wybierz typ dla zadań, które mają teraz ten typ',
  'The sprint needs a name': 'Sprint musi mieć nazwę',
  'Only a sprint that has not started can be started': 'Rozpocząć można tylko sprint, który jeszcze się nie zaczął',
  'The sprint cannot end before it starts': 'Sprint nie może kończyć się przed swoim początkiem',
  'Only a running sprint can be completed': 'Zakończyć można tylko trwający sprint',
  'Unfinished issues can only move to an open sprint of the same project': 'Nieukończone zadania można przenieść tylko do niezakończonego sprintu tego samego projektu',
  "A completed sprint is part of the project's history and cannot be deleted": 'Zakończony sprint jest częścią historii projektu i nie można go usunąć',
  'The version needs a name': 'Wersja musi mieć nazwę',
  'Unfinished issues can only move to an unreleased version of the same project': 'Nieukończone zadania można przenieść tylko do niewydanej wersji tego samego projektu',
  'The filter needs a name': 'Filtr musi mieć nazwę',

  // ---- the query language ----
  'Unterminated quoted text': 'Niezamknięty cudzysłów',
  'Unexpected "!"': 'Nieoczekiwany znak „!”',
  'Expected ")"': 'Oczekiwano „)”',
  'Expected AND, OR or ORDER BY': 'Oczekiwano AND, OR albo ORDER BY',
  'Expected a field to order by': 'Oczekiwano pola, według którego sortować',
  'Expected a field name': 'Oczekiwano nazwy pola',
  'Expected EMPTY after IS': 'Po IS oczekiwano EMPTY',
  'Expected a value': 'Oczekiwano wartości',
  'Category is one of: todo, "in progress", done': 'Kategoria to jedna z wartości: todo, "in progress", done',
  'Priority is one of: Highest, High, Medium, Low, Lowest': 'Priorytet to jedna z wartości: Highest, High, Medium, Low, Lowest',
  'Points are compared with a number': 'Punkty porównuje się z liczbą',

  // ---- AI agents: errors ----
  Proposal: 'propozycja',
  // Both an agent's run („zlecenie”) and a test run („przebieg”) are a "Run" to the server.
  Run: 'zlecenie albo przebieg testów',
  person: 'osoba',
  priority: 'priorytet',
  model: 'model',
  repository: 'repozytorium',
  'branch name': 'nazwa gałęzi',
  'pull request address': 'adres pull requesta',
  'runner name': 'nazwa runnera',
  'The comment': 'komentarz',
  'The summary': 'tytuł',
  "A subtask's summary": 'tytuł podzadania',
  'The title of the pull request': 'tytuł pull requesta',
  'there are none': 'brak dozwolonych wartości',
  'no details': 'brak szczegółów',
  'The person who asked for this run is no longer active': 'Konto osoby, która zleciła tę pracę, nie jest już aktywne',
  'The proposal changes no field (it may set summary, description, priority, labels, points, assignee, dueDate)':
    'Propozycja nie zmienia żadnego pola (może ustawić: summary, description, priority, labels, points, assignee, dueDate)',
  'This proposal has already been decided': 'Decyzja w sprawie tej propozycji już zapadła',
  "This is decided in the runner's app, where the changes can be looked at first": 'Decyzja zapada w aplikacji runnera, gdzie można najpierw obejrzeć zmiany',
  'A person decides this proposal here; a runner cannot settle it': 'O tej propozycji decyduje tutaj człowiek; runner nie może jej rozstrzygnąć',
  'Only a runner that has the code can propose a push or a pull request': 'Wypchnięcie gałęzi albo pull request może zaproponować tylko runner, który ma kod',
  'That is not a branch name': 'To nie jest nazwa gałęzi',
  'The note is too long': 'Notatka jest za długa',
  'The description of the pull request is too long': 'Opis pull requesta jest za długi',
  'The result is too long': 'Wynik jest za długi',
  'The run was cancelled': 'Zlecenie zostało anulowane',
  'Applied at once: the project trusts this kind of proposal': 'Zastosowano od razu: projekt ufa propozycjom tego rodzaju',
  'A run ends as "succeeded" or "failed"': 'Zlecenie kończy się stanem "succeeded" albo "failed"',
  'That runner has not announced itself yet; send a heartbeat first': 'Ten runner jeszcze się nie zgłosił; najpierw wyślij heartbeat',
  'That is not a run state': 'To nie jest stan zlecenia',
  'The answer is empty': 'Odpowiedź jest pusta',
  'The run has already ended': 'Zlecenie już się zakończyło',
  'The run has already ended; only its branch and pull request can still be recorded': 'Zlecenie już się zakończyło; można jeszcze zapisać tylko jego gałąź i pull request',
  'Only the runner that claimed this run may write to it': 'Do tego zlecenia może pisać tylko runner, który je podjął',
  'The run waits for an answer to its question; give it as the answer': 'Zlecenie czeka na odpowiedź na swoje pytanie; prześlij ją jako odpowiedź',
  'Answers come from people': 'Odpowiedzi pochodzą od ludzi',
  'An event needs a text of reasonable length': 'Zdarzenie musi mieć tekst o rozsądnej długości',
  'The pull request address must start with https://': 'Adres pull requesta musi zaczynać się od https://',
  'Tokens and cost cannot be negative': 'Liczba tokenów i koszt nie mogą być ujemne',
  'A runner cannot put a run back in the queue': 'Runner nie może odesłać zlecenia z powrotem do kolejki',
  'A runner id is 1 to 100 characters without spaces': 'Identyfikator runnera to od 1 do 100 znaków bez spacji',
  "That runner id belongs to somebody else's runner": 'Ten identyfikator należy do runnera innej osoby',
  'The API address must start with https://': 'Adres API musi zaczynać się od https://',
  'That is not one of the models the server can use': 'To nie jest żaden z modeli, których może używać serwer',
  'The monthly cap is an amount in US dollars': 'Miesięczny limit to kwota w dolarach USA',
  'The model declined to work on this request': 'Model odmówił pracy nad tym zleceniem',
  "The model's answer was cut off before it was complete": 'Odpowiedź modelu została ucięta, zanim była kompletna',

  // ---- AI agents: what a run reports, and what its requester is told ----
  Server: 'Serwer',
  'Reading the issue': 'Czytam zadanie',
  Cancelled: 'Anulowano',
  'Asked to stop': 'Poproszono o zatrzymanie',
  'The runner stopped answering; the run is queued again': 'Runner przestał odpowiadać; zlecenie wróciło do kolejki',
  'The server restarted; the run is queued again': 'Serwer został uruchomiony ponownie; zlecenie wróciło do kolejki',
  'a comment': 'komentarz',
  'a change of status': 'zmiana statusu',
  "changes to the issue's fields": 'zmiany w polach zadania',
  subtasks: 'podzadania',
  'a new issue': 'nowe zadanie',
  'a work log': 'wpis czasu pracy',
  'pushing the branch': 'wypchnięcie gałęzi',
  'opening a pull request': 'otwarcie pull requesta',
  'the agent run failed': 'zlecenie agenta nie powiodło się',
  'no summary': 'brak podsumowania',

  // ---- knowledge base ----
  Page: 'strona',
  Space: 'przestrzeń',
  'A page needs a title': 'Strona musi mieć tytuł',
  'The page is larger than 1 MB; split it into several pages': 'Strona jest większa niż 1 MB; podziel ją na kilka stron',
  'The parent page is in another space': 'Strona nadrzędna jest w innej przestrzeni',
  'The parent page does not exist': 'Strona nadrzędna nie istnieje',
  'This page mirrors Confluence; detach it to edit': 'Ta strona jest kopią z Confluence; odłącz ją, żeby edytować',
  'This page mirrors Confluence, which decides where it sits; detach it to move it':
    'Ta strona jest kopią z Confluence i to Confluence decyduje o jej miejscu; odłącz ją, żeby ją przenieść',
  'A page cannot be moved under itself or one of its own descendants': 'Strony nie można przenieść pod nią samą ani pod żadną z jej stron podrzędnych',
  'This page has pages under it; delete them too, or move them first': 'Ta strona ma strony podrzędne; usuń je razem z nią albo najpierw je przenieś',
  'Choose at least one task': 'Wybierz co najmniej jeden punkt',
  'There is no such template': 'Nie ma takiego szablonu',
  'A space key is 2 to 10 letters and digits, starting with a letter': 'Klucz przestrzeni to od 2 do 10 liter i cyfr, zaczyna się od litery',

  // ---- Confluence import ----
  Import: 'import',
  'The Confluence address must start with https://': 'Adres Confluence musi zaczynać się od https://',
  'That does not look like a Confluence address, e.g. https://example.atlassian.net/wiki': 'To nie wygląda na adres Confluence, np. https://example.atlassian.net/wiki',
  'Set the Confluence address and token first': 'Najpierw ustaw adres i token Confluence',
  'Confluence rejected the token': 'Confluence odrzuciło token',
  "Confluence refused access; the token's account may not be allowed to see this": 'Confluence odmówiło dostępu; konto, do którego należy token, może nie mieć do tego uprawnień',
  'Confluence has nothing at that address; check the address and the space key': 'Pod tym adresem Confluence nic nie ma; sprawdź adres i klucz przestrzeni',
  'Confluence keeps sending more pages of results than a space can have': 'Confluence wciąż przysyła kolejne strony wyników, więcej, niż może mieć jedna przestrzeń',
  'Name the Confluence space to import': 'Podaj przestrzeń Confluence do zaimportowania',
  'The address points to another server; enter the token for it': 'Adres wskazuje inny serwer; podaj token do niego',
  'Confluence sent no body for this page': 'Confluence nie przysłało treści tej strony',
  'The page could not be converted to Markdown': 'Nie udało się przekształcić strony na Markdown',

  // ---- Jira import ----
  Upload: 'przesłany plik',
  // Names the import gives to what it creates stay as they are.
  Jira: 'Jira',
  Task: 'Task',
  'The Jira address must start with https://': 'Adres Jiry musi zaczynać się od https://',
  'That does not look like a Jira address, e.g. https://example.atlassian.net': 'To nie wygląda na adres Jiry, np. https://example.atlassian.net',
  'Set the Jira address and token first': 'Najpierw ustaw adres i token Jiry',
  'Jira rejected the token': 'Jira odrzuciła token',
  "Jira refused access; the token's account may not be allowed to see this": 'Jira odmówiła dostępu; konto, do którego należy token, może nie mieć do tego uprawnień',
  'Jira has nothing at that address; check the address and the project key': 'Pod tym adresem Jira nic nie ma; sprawdź adres i klucz projektu',
  'the request was not understood': 'żądanie nie zostało zrozumiane',
  'Jira keeps sending more pages than a listing can have': 'Jira wciąż przysyła kolejne strony, więcej, niż może mieć jedna lista',
  'That import has already finished': 'Ten import już się zakończył',
  'Name the Jira project by its key, e.g. WEB': 'Podaj klucz projektu w Jirze, np. WEB',
  'The file has no column with issue keys. Export from Jira with “Export CSV (all fields)”, or choose which column holds the keys.':
    'Plik nie ma kolumny z kluczami zadań. Wyeksportuj dane z Jiry przez „Eksport → CSV (wszystkie pola)” albo wskaż, która kolumna zawiera klucze.',
  'No row of the file has an issue key such as WEB-12': 'Żaden wiersz pliku nie ma klucza zadania takiego jak WEB-12',
  'Jira sent it without a key or a creation date': 'Jira przysłała je bez klucza albo bez daty utworzenia',
  'An issue with this key already exists here and did not come from Jira': 'Zadanie o tym kluczu już tu istnieje i nie pochodzi z Jiry',
  'Edited here after it was imported; the newer version in Jira was not applied': 'Edytowane tutaj po imporcie; nowsza wersja z Jiry nie została zastosowana',
  'The server stopped while this import was running. Run it again: what was already imported is kept.':
    'Serwer zatrzymał się w trakcie tego importu. Uruchom go ponownie: to, co już zaimportowano, zostaje.',
  'The import stopped unexpectedly; what it had imported is kept': 'Import zatrzymał się nieoczekiwanie; to, co zaimportował, zostaje',

  // ---- GitHub ----
  Repository: 'repozytorium',
  'The GitHub API address must start with https://': 'Adres API GitHuba musi zaczynać się od https://',
  'No GitHub token is configured (Settings → GitHub)': 'Nie ustawiono tokenu GitHuba (Ustawienia → GitHub)',
  'GitHub rejected the token': 'GitHub odrzucił token',
  'GitHub kept failing': 'GitHub wciąż zwracał błędy',
  'GitHub did not say where the default branch is': 'GitHub nie podał, gdzie jest gałąź domyślna',
  'GitHub did not say which pull request it opened': 'GitHub nie podał, który pull request otworzył',
  'That operator cannot be used with "drift"': 'Tego operatora nie można użyć z polem „drift”',
  'A repository is written owner/name, for example acme/web-shop': 'Repozytorium zapisuje się jako właściciel/nazwa, na przykład acme/web-shop',
  "That repository is not connected to this issue's project": 'To repozytorium nie jest podłączone do projektu tego zadania',
  'A branch name may contain letters, digits, dashes, dots, underscores and slashes': 'Nazwa gałęzi może zawierać litery, cyfry, myślniki, kropki, podkreślenia i ukośniki',
  'A pull request needs a branch other than the one it merges into': 'Pull request wymaga innej gałęzi niż ta, do której ma zostać zmergowany',
  'The title or the description is too long': 'Tytuł albo opis jest za długi',
  'Missing X-GitHub-Event': 'Brak nagłówka X-GitHub-Event',
  'No webhook secret is configured': 'Nie ustawiono sekretu webhooka',
  'The signature does not match': 'Podpis się nie zgadza',
  'The payload is not JSON': 'Treść nie jest poprawnym JSON-em',

  // ---- GitHub: the log of webhook deliveries ----
  pong: 'pong',
  'no repository in the payload': 'brak repozytorium w treści zdarzenia',
  'this repository is not connected to a project': 'to repozytorium nie jest podłączone do żadnego projektu',
  'not a branch push': 'push nie dotyczy gałęzi',
  'not a branch': 'to nie gałąź',
  'no branch name': 'brak nazwy gałęzi',
  'no pull request in the payload': 'brak pull requesta w treści zdarzenia',
  'incomplete pull request': 'niekompletne dane pull requesta',
  'review recorded': 'zapisano review',
  'no commit in the payload': 'brak commita w treści zdarzenia',
  'check state not understood': 'nierozpoznany stan testów',
  'event not handled': 'zdarzenie nieobsługiwane',
  passing: 'przechodzą',
  failing: 'nie przechodzą',
  running: 'trwają',
  'issue gone': 'zadanie już nie istnieje',

  // ---- tests (e2e), reports, timeline ----
  Test: 'test',
  '"key" is required': 'Pole "key" jest wymagane',
  '"projectKey" is required': 'Pole "projectKey" jest wymagane',
  'This is not an e2e report: it has no "run". Post the document the json reporter prints, or .e2e/report.json.':
    'To nie jest raport e2e: nie ma w nim pola "run". Wyślij dokument, który wypisuje reporter json, albo plik .e2e/report.json.',
  'The JUnit file is not UTF-8 text': 'Plik JUnit nie jest tekstem w UTF-8',
  'This is not a JUnit file: its root is neither <testsuites> nor <testsuite>': 'To nie jest plik JUnit: jego elementem głównym nie jest ani <testsuites>, ani <testsuite>',
  'The request has no body; post the run report': 'Żądanie nie ma treści; wyślij raport z przebiegu',
  'The unit is "points" or "issues"': 'Jednostka to "points" albo "issues"',
  '"project" is required': 'Pole "project" jest wymagane',
  'The start date is after the due date': 'Data rozpoczęcia jest późniejsza niż termin',
};

export const patterns: Record<string, string> = {
  // ---- with a thing's name in them ----
  '{what} not found': 'Nie znaleziono: {t:what}',
  '{what} needs a name of 1 to 60 characters': 'Nazwa musi mieć od 1 do 60 znaków ({t:what})',
  'That {what} does not belong to this project': 'Ten element nie należy do tego projektu: {t:what}',
  'There is no {what} called "{name}"': 'Nie ma takiej nazwy: „{name}” ({t:what})',
  '{what} is empty': 'Puste pole: {t:what}',
  'That is not a valid {what}': 'Nieprawidłowa wartość: {t:what}',
  'Unknown {what} "{given}" ({valid})': 'Nieznana wartość „{given}” ({t:what}; {t:valid})',
  'valid: {list}': 'dozwolone: {list}',

  // ---- an error about one issue of several, and what an agent's requester is told ----
  '{key}: {text}': '{key}: {t:text}',

  // ---- notifications ----
  '{key} was assigned to you: {summary}': 'Przypisano Ci {key}: {summary}',
  'You were mentioned in {key}: {summary}': 'Wspomniano o Tobie w {key}: {summary}',
  '{author} mentioned you in {key}: {text}': '{t:author} wspomina o Tobie w {key}: {text}',
  '{author} commented on {key}: {text}': '{t:author} komentuje {key}: {text}',
  '{key} moved to {status}: {summary}': 'Zmiana statusu {key} na „{status}”: {summary}',
  'the agent finished: {outcome}': 'agent zakończył pracę: {t:outcome}',
  'the agent asks: {question}': 'agent pyta: {question}',
  'the agent proposes {what}': 'propozycja agenta: {t:what}',

  // ---- issues, projects, sprints ----
  'Enable the "{module}" module first': 'Najpierw włącz moduł „{module}”',
  'The "{module}" module depends on this one': 'Od tego modułu zależy moduł „{module}”',
  '"{text}" is not a date (expected YYYY-MM-DD)': '„{text}” nie jest datą (oczekiwany format: RRRR-MM-DD)',
  'The summary is longer than {max} characters': 'Tytuł ma więcej niż {max} znaków',
  '{key} does not exist': 'Zadanie {key} nie istnieje',
  '{key} is not in this project': 'Zadanie {key} nie należy do tego projektu',
  'The workflow does not allow moving from "{from}" to "{to}"': 'Workflow nie pozwala na przejście z „{from}” do „{to}”',
  'The key {projectKey} is already taken': 'Klucz {projectKey} jest już zajęty',
  '"{name}" is still running; complete it first': 'Sprint „{name}” jeszcze trwa; najpierw go zakończ',

  // ---- the query language ----
  'Expected an operator after "{field}"': 'Po „{field}” oczekiwano operatora',
  'That operator cannot be used with "{field}"': 'Tego operatora nie można użyć z polem „{field}”',
  'A function cannot be used with "{field}"': 'Z polem „{field}” nie można użyć funkcji',
  'Unknown function {name}()': 'Nieznana funkcja {name}()',
  'Unknown field "{field}"': 'Nieznane pole „{field}”',
  'Cannot order by "{field}"': 'Nie można sortować według „{field}”',
  'There is nobody called "{name}"': 'Nie ma osoby o nazwie „{name}”',
  'There is no sprint called "{name}"': 'Nie ma sprintu o nazwie „{name}”',
  '"{field}" needs a date such as 2026-11-30, -7d or startOfWeek()': 'Pole „{field}” wymaga daty, na przykład 2026-11-30, -7d albo startOfWeek()',
  '"{text}" is not an issue key': '„{text}” nie jest kluczem zadania',
  '"{word}" is not one of: {allowed}': '„{word}” nie jest jedną z dozwolonych wartości: {allowed}',
  '"{field}" takes a word such as failing or passing': 'Pole „{field}” przyjmuje słowo takie jak failing albo passing',
  '"{field}" takes a word such as open or failing': 'Pole „{field}” przyjmuje słowo takie jak open albo failing',
  '"{word}" is not a kind of drift ({kinds})': '„{word}” nie jest rodzajem rozbieżności ({kinds})',

  // ---- AI agents: errors ----
  'Invalid arguments: {error}': 'Nieprawidłowe argumenty: {error}',
  'The query could not be understood at character {position}: {message}': 'Nie udało się zrozumieć zapytania (znak {position}): {t:message}',
  'The model provider rejected the API key ({message})': 'Dostawca modelu odrzucił klucz API ({t:message})',
  'The model provider answered {status}: {message}': 'Dostawca modelu odpowiedział kodem {status}: {t:message}',
  'The model provider kept failing ({failure})': 'Dostawca modelu wciąż zwracał błędy ({t:failure})',
  'status {status}': 'kod {status}',
  'unreachable: {error}': 'brak połączenia: {error}',
  "The proposal's payload is not what its kind takes: {error}": 'Treść propozycji nie pasuje do jej rodzaju: {error}',
  'A breakdown has 1 to {max} items': 'Podział na podzadania ma od 1 do {max} pozycji',
  '{key} is a subtask itself and cannot have subtasks': '{key} samo jest podzadaniem i nie może mieć podzadań',
  'The instructions are longer than {max} characters': 'Wskazówki dla agenta mają więcej niż {max} znaków',
  'There is no tool called "{name}"': 'Nie ma narzędzia o nazwie „{name}”',
  'The run used more than {max} tokens and was stopped': 'Zlecenie zużyło ponad {max} tokenów i zostało zatrzymane',

  // ---- AI agents: what a run reports ----
  'Taken by {name}': 'Podjęte przez: {t:name}',
  'Asking {model}': 'Pytam model {model}',
  'Asking {model} again (step {step})': 'Pytam model {model} ponownie (krok {step})',
  'Proposed {what}': 'Propozycja: {t:what}',
  'Approved: {what}': 'Zatwierdzono: {t:what}',
  'Rejected: {what}': 'Odrzucono: {t:what}',
  "Done in the runner's app: {what}": 'Wykonano w aplikacji desktopowej: {t:what}',
  "Declined in the runner's app: {what}": 'Odrzucono w aplikacji desktopowej: {t:what}',
  'Could not apply {what}: {why}': 'Nie udało się zastosować ({t:what}): {t:why}',
  'A proposal was refused: {why}': 'Propozycja nie została przyjęta: {t:why}',

  // ---- knowledge base ----
  'The title is longer than {max} characters': 'Tytuł ma więcej niż {max} znaków',
  'The description is longer than {max} characters': 'Opis ma więcej niż {max} znaków',
  'A space needs a name of 1 to {max} characters': 'Przestrzeń musi mieć nazwę o długości od 1 do {max} znaków',
  'The space key {spaceKey} is already taken': 'Klucz przestrzeni {spaceKey} jest już zajęty',
  'The space {space} already exists and is not a {source} import; choose another key': 'Przestrzeń {space} już istnieje i nie pochodzi z importu ({source}); wybierz inny klucz',
  'Someone else saved version {version} of this page; reload it to see their changes': 'Ktoś inny zapisał wersję {version} tej strony; wczytaj ją ponownie, żeby zobaczyć zmiany',
  'Someone else saved version {version} of this page; reload it and choose the tasks again':
    'Ktoś inny zapisał wersję {version} tej strony; wczytaj ją ponownie i jeszcze raz wybierz punkty',
  'The page has no line {number}': 'Strona nie ma wiersza {number}',
  'Line {number} is not an open task': 'Wiersz {number} nie jest otwartym punktem do zrobienia',
  'Line {number} already belongs to {issue}': 'Wiersz {number} należy już do zadania {issue}',

  // ---- Confluence import ----
  'Could not reach Confluence: {error}': 'Nie udało się połączyć z Confluence: {error}',
  'Confluence sent an answer that is not JSON ({error}); check the address': 'Confluence przysłało odpowiedź, która nie jest JSON-em ({error}); sprawdź adres',
  'Confluence answered with status {status}': 'Confluence odpowiedziało kodem {status}',
  'Confluence kept answering with status {status}; try again later': 'Confluence wciąż odpowiadało kodem {status}; spróbuj ponownie później',
  'The Confluence space {space} has no page with the id {id}': 'W przestrzeni Confluence {space} nie ma strony o identyfikatorze {id}',
  '"{spaceKey}" cannot be a space key here (2 to 10 letters and digits, starting with a letter); choose a target space key':
    '„{spaceKey}” nie może być tutaj kluczem przestrzeni (od 2 do 10 liter i cyfr, zaczyna się od litery); wybierz klucz przestrzeni docelowej',
  'The space {space} already exists and is not a Confluence import; choose another target space key':
    'Przestrzeń {space} już istnieje i nie jest importem z Confluence; wybierz inny klucz przestrzeni docelowej',
  'Connected to {address}': 'Połączono z {address}',

  // ---- Jira import: errors ----
  'Could not reach Jira: {error}': 'Nie udało się połączyć z Jirą: {error}',
  'Jira sent an answer that is not JSON ({error}); check the address': 'Jira przysłała odpowiedź, która nie jest JSON-em ({error}); sprawdź adres',
  'Jira answered: {message}': 'Jira odpowiedziała: {t:message}',
  'Jira answered with status {status}': 'Jira odpowiedziała kodem {status}',
  'Jira kept answering with status {status}; try again later': 'Jira wciąż odpowiadała kodem {status}; spróbuj ponownie później',
  '"{projectKey}" cannot be a project key here (2 to 10 letters and digits, starting with a letter), and issue keys must stay as they are':
    '„{projectKey}” nie może być tutaj kluczem projektu (od 2 do 10 liter i cyfr, zaczyna się od litery), a klucze zadań muszą pozostać bez zmian',
  "A project with the key {projectKey} already exists here. Choose “import into the existing project” to add Jira's issues to it.":
    'Projekt o kluczu {projectKey} już tu istnieje. Wybierz „Importuj do istniejącego projektu”, żeby dodać do niego zadania z Jiry.',
  'An import of {projectKey} is still running; wait for it or cancel it': 'Import projektu {projectKey} jeszcze trwa; poczekaj na jego koniec albo go anuluj',
  'The file has no issues of {projectKey}': 'W pliku nie ma zadań projektu {projectKey}',
  'Connected to {title} at {address}': 'Połączono: {title} ({address})',

  // ---- Jira import: the report ----
  'Row {line} has no issue key': 'Wiersz {line} nie ma klucza zadania',
  'Row {line}: "{text}" is not an issue key': 'Wiersz {line}: „{text}” nie jest kluczem zadania',
  '{project}-{number} appears more than once; the first row counts': '{project}-{number} występuje więcej niż raz; liczy się pierwszy wiersz',
  '{key}: the creation date "{value}" was not understood; the time of the import is used': '{key}: nie udało się odczytać daty utworzenia „{value}”; użyto czasu importu',
  '{key}: its parent is not in the file': '{key}: zadania nadrzędnego nie ma w pliku',
  '{key}: {parent} cannot be its parent here (epics hold stories, tasks and bugs; those hold subtasks)':
    '{key}: {parent} nie może być tutaj zadaniem nadrzędnym (epiki zawierają historyjki, zadania i błędy, a te zawierają podzadania)',
  '{key}: its parent {parent} is in another project': '{key}: zadanie nadrzędne {parent} jest w innym projekcie',
  '{key}: its parent {parent} was not imported': '{key}: zadanie nadrzędne {parent} nie zostało zaimportowane',
  '{key}: it has {count} fix versions; the first, “{name}”, is kept': '{key}: ma kilka wersji ({count}); zostaje pierwsza, „{name}”',
  '{key}: {points} story points is outside what an estimate can be here': '{key}: {points} pkt to wartość spoza zakresu, jaki może mieć tutaj szacunek',
  'only the first {count} comments came through: {error}': 'udało się pobrać tylko początkowe komentarze ({count}): {t:error}',
  'only the first {count} work logs came through: {error}': 'udało się pobrać tylko początkowe wpisy czasu pracy ({count}): {t:error}',
  'its history is incomplete: {error}': 'historia jest niepełna: {t:error}',
  'The issue type “{jiraName}” was not in the mapping; it was added as “{name}”': 'Typu zadania „{jiraName}” nie było w mapowaniu; został dodany jako „{name}”',
  'The type “{name}” already exists here on the {level} level; that level is kept': 'Typ „{name}” już tu istnieje na poziomie „{level}”; ten poziom zostaje',
  "The status “{jiraName}” is not in the project's workflow any more; it was kept for the history, off the board":
    'Statusu „{jiraName}” nie ma już w workflow projektu; został zachowany na potrzeby historii, poza tablicą',
  'Sprint {id}': 'Sprint {id}',
  'Jira runs “{running}” and “{name}” at the same time; a project here runs one sprint at a time, so “{planned}” is planned, not running':
    'W Jirze sprinty „{running}” i „{name}” trwają jednocześnie; tutaj w projekcie trwa jeden sprint naraz, więc „{planned}” jest zaplanowany, a nie trwający',
  'Links of the type “{type}” were imported as “relates to”': 'Powiązania typu „{type}” zaimportowano jako „dotyczy”',
  '{count} links point to issues that are not here (other projects, or issues that were not imported)':
    'Powiązania prowadzące do zadań, których tu nie ma (inne projekty albo zadania, których nie zaimportowano): {count}',

  // ---- GitHub ----
  'GitHub is unreachable: {error}': 'Nie udało się połączyć z GitHubem: {error}',
  'GitHub answered {status} to {method} {path}: {message}': 'GitHub odpowiedział kodem {status} na {method} {path}: {t:message}',
  '{owner}/{name} is already connected to project {project}': 'Repozytorium {owner}/{name} jest już podłączone do projektu {project}',

  // ---- GitHub: the log of webhook deliveries ----
  'ignored: {reason}': 'pominięto: {t:reason}',
  'branch {branch} created': 'utworzono gałąź {branch}',
  'branch {branch} deleted': 'usunięto gałąź {branch}',
  'branch {branch}: {count} new commits linked': 'gałąź {branch}: powiązano nowe commity ({count})',
  'pull request #{number} {action}': 'pull request #{number}: {action}',
  'checks {state}': 'testy {t:state}',
  'failed: {error}': 'nie udało się: {t:error}',
  '{key} moved ({trigger})': '{key}: zmieniono status ({trigger})',
  '{key} moved by commit': '{key}: status zmieniony przez commit',
  '{key} commented': '{key}: dodano komentarz',
  '{key} logged {minutes}m': '{key}: zapisano czas pracy ({minutes} min)',
  '{key}: no status called "{name}"': '{key}: nie ma statusu o nazwie „{name}”',

  // ---- tests (e2e) ----
  'The report is not valid JSON: {error}': 'Raport nie jest poprawnym JSON-em: {error}',
  'Unknown report schema "{version}"; expected report-1': 'Nieznany schemat raportu „{version}”; oczekiwano report-1',
  'The JUnit file is not valid XML: {error}': 'Plik JUnit nie jest poprawnym XML-em: {error}',
  '"meta" is not understood: {error}': 'Nie udało się odczytać pola "meta": {error}',
};
