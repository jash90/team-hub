/** Polish for the Confluence import's phrases. */
export const polish: Record<string, string> = {
  // connection
  Connection: 'Połączenie',
  'The address and one token for the whole instance. The token is stored encrypted and never shown again; imports can read whatever its account can read in Confluence.':
    'Adres i jeden token dla całej instancji. Token jest przechowywany w postaci zaszyfrowanej i nigdy więcej nie jest pokazywany; importy mogą odczytać to, co w Confluence może odczytać jego konto.',
  'Confluence address': 'Adres Confluence',
  'Cloud: https://your-site.atlassian.net/wiki (with /wiki). Server and Data Center: the address people open, with its context path.':
    'Cloud: https://twoja-witryna.atlassian.net/wiki (z /wiki). Server i Data Center: adres, który otwierają ludzie, razem ze ścieżką kontekstu.',
  'Cloud: the e-mail of the account the API token belongs to. Leave empty for a personal access token (Server, Data Center).':
    'Cloud: e-mail konta, do którego należy token API. Zostaw puste dla osobistego tokenu dostępu (Server, Data Center).',
  Token: 'Token',
  'A token ending in {hint} is stored. Leave blank to keep it.': 'Zapisany jest token kończący się na {hint}. Zostaw puste, żeby go zachować.',
  'No token is stored.': 'Nie zapisano tokenu.',
  'Test connection': 'Sprawdź połączenie',
  'Testing…': 'Sprawdzam…',
  'Save first: the test uses what is stored': 'Najpierw zapisz: sprawdzane jest to, co zapisane',
  'It works.': 'Działa.',
  'It does not work.': 'Nie działa.',
  'Remove token': 'Usuń token',
  'Token removed': 'Token usunięty',

  // import
  'Import a space': 'Importuj przestrzeń',
  'Copies the pages of a Confluence space into a knowledge base space as a read-only mirror. Running it again later brings the copy up to date.':
    'Kopiuje strony przestrzeni Confluence do przestrzeni bazy wiedzy jako kopię tylko do odczytu. Ponowne uruchomienie później aktualizuje kopię.',
  'Store the address and a token above first.': 'Najpierw zapisz powyżej adres i token.',
  'Reload spaces': 'Wczytaj przestrzenie ponownie',
  'Asking Confluence for its spaces…': 'Pytam Confluence o przestrzenie…',
  'Confluence space': 'Przestrzeń Confluence',
  'Choose a space…': 'Wybierz przestrzeń…',
  'Into the space with the key': 'Do przestrzeni o kluczu',
  'Created if it does not exist. 2–10 letters and digits. A space that is not a Confluence import is never overwritten.':
    'Zostanie utworzona, jeśli nie istnieje. 2–10 liter i cyfr. Przestrzeń, która nie jest importem z Confluence, nigdy nie zostanie nadpisana.',
  'Only under the page with this id (optional)': 'Tylko pod stroną o tym identyfikatorze (opcjonalnie)',
  'The number in the address of the page in Confluence. Empty imports the whole space.': 'Liczba z adresu strony w Confluence. Puste pole importuje całą przestrzeń.',
  'Leave out (optional)': 'Pomiń (opcjonalnie)',
  'One page per line, by title or by id. Everything under such a page is left out too.': 'Jedna strona w wierszu, tytuł albo identyfikator. Wszystko pod taką stroną też zostanie pominięte.',
  Import: 'Importuj',
  'Importing…': 'Importuję…',
  'Importing {key}…': 'Importuję {key}…',
  'Every page is fetched and converted inside this one request, so this can take a while for a large space. Keep this tab open.':
    'Każda strona jest pobierana i przekształcana w ramach tego jednego żądania, więc przy dużej przestrzeni może to chwilę potrwać. Nie zamykaj tej karty.',
  'Import finished': 'Import zakończony',
  'Imported into': 'Zaimportowano do',

  // report
  '{count} created': 'utworzone: {count}',
  '{count} updated': 'zaktualizowane: {count}',
  '{count} unchanged': 'bez zmian: {count}',
  '{count} skipped': 'pominięte: {count}',
  '{count} gone from Confluence': 'nie ma już w Confluence: {count}',
  '{count} failed': 'nieudane: {count}',
  'Excluded pages with everything under them, and pages detached here': 'Strony wykluczone razem ze wszystkim pod nimi oraz strony odłączone tutaj',
  'The space has no pages.': 'Przestrzeń nie ma stron.',
  'Pages that could not be stored': 'Strony, których nie udało się zapisać',
  '(no title)': '(bez tytułu)',
  'Pages Confluence no longer has (kept here as they are)': 'Strony, których nie ma już w Confluence (tutaj zostają bez zmian)',

  // imports
  'Imported spaces': 'Zaimportowane przestrzenie',
  'A sync fetches the space again: new pages are added, changed ones get a new version, unchanged and detached ones are left alone.':
    'Synchronizacja pobiera przestrzeń ponownie: nowe strony są dodawane, zmienione dostają nową wersję, niezmienione i odłączone zostają nietknięte.',
  'Nothing has been imported yet.': 'Niczego jeszcze nie zaimportowano.',
  'from the Confluence space {key}': 'z przestrzeni Confluence {key}',
  'under page {id}': 'pod stroną {id}',
  'leaving out: {list}': 'z pominięciem: {list}',
  'Sync now': 'Synchronizuj teraz',
  'Syncing…': 'Synchronizuję…',
  'Syncing {key}…': 'Synchronizuję {key}…',
  '{key} is up to date': '{key} jest aktualna',
  'Last synced {when}': 'Ostatnia synchronizacja: {when}',
  'Never synced': 'Jeszcze nie synchronizowano',
};
