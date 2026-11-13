/** Polish for the reports module's phrases. */
export const polish: Record<string, string> = {
  Reports: 'Raporty',
  'Drawn from the history of the issues: every change of status, sprint and estimate counts from the moment it was made.':
    'Powstają z historii zadań: każda zmiana statusu, sprintu i wyceny liczy się od chwili, w której ją wprowadzono.',
  'This project works as a kanban board, without sprints. Burndown and velocity measure sprints, so they are not shown here; flow and cycle time say the most about a kanban team.':
    'Ten projekt pracuje na tablicy kanban, bez sprintów. Wykres spalania i prędkość mierzą sprinty, więc ich tu nie ma; o zespole kanban najwięcej mówią przepływ i czas realizacji.',
  Burndown: 'Wykres spalania',
  Velocity: 'Prędkość',
  'Cumulative flow': 'Przepływ skumulowany',
  'Created vs resolved': 'Utworzone a rozwiązane',
  'Cycle time': 'Czas realizacji',
  Workload: 'Obciążenie',

  // controls
  Period: 'Okres',
  '{count} days': '{count} dni',
  'Measure in': 'Jednostka',
  Points: 'Punkty',
  Issues: 'Zadania',
  Sprint: 'Sprint',
  Measure: 'Miara',
  'From creation': 'Od utworzenia',
  'From start of work': 'Od rozpoczęcia pracy',
  'Show the numbers': 'Pokaż liczby',

  // burndown
  'Work left in the sprint at the end of each day. Under the dashed line the sprint is ahead of a steady pace; a step in the scope line is work added or removed after the start.':
    'Praca, która została w sprincie na koniec każdego dnia. Pod linią przerywaną sprint wyprzedza równe tempo; skok linii zakresu to praca dodana lub usunięta po starcie.',
  'No sprint has been started yet': 'Żaden sprint jeszcze nie wystartował',
  'Start a sprint from the backlog. Its burndown appears here from the first day.': 'Rozpocznij sprint w backlogu. Jego wykres spalania pojawi się tutaj od pierwszego dnia.',
  'This sprint has no days to show yet': 'Ten sprint nie ma jeszcze dni do pokazania',
  Committed: 'Zaplanowane',
  Remaining: 'Pozostało',
  Completed: 'Ukończone',
  'Scope changes': 'Zmiany zakresu',
  'added / removed': 'dodane / usunięte',
  Ideal: 'Równe tempo',
  Scope: 'Zakres',
  'added that day': 'dodane tego dnia',
  'removed that day': 'usunięte tego dnia',
  Day: 'Dzień',
  Added: 'Dodano',
  Removed: 'Usunięto',

  // velocity
  'What each completed sprint took on at its start and what it finished. The line is the average of finished work: a fair guide to how much to plan next.':
    'Ile każdy zakończony sprint wziął na starcie i ile ukończył. Linia to średnia ukończonej pracy: dobra wskazówka, ile planować na następny.',
  'No sprint has been completed yet': 'Żaden sprint nie został jeszcze zakończony',
  'Velocity compares completed sprints; the first one shows up here when it is completed.': 'Prędkość porównuje zakończone sprinty; pierwszy pojawi się tutaj, gdy zostanie zakończony.',
  'Average completed': 'Średnio ukończone',
  'per sprint': 'na sprint',
  'Last sprint': 'Ostatni sprint',
  'completed / committed': 'ukończone / zaplanowane',
  'Committed points': 'Zaplanowane punkty',
  'Completed points': 'Ukończone punkty',
  'Committed issues': 'Zaplanowane zadania',
  'Completed issues': 'Ukończone zadania',

  // cumulative flow
  'How many issues were in each stage at the end of each day. A band that keeps widening is where work piles up; the top edge is everything ever created.':
    'Ile zadań było na każdym etapie na koniec każdego dnia. Pas, który wciąż się poszerza, to miejsce, gdzie praca się piętrzy; górna krawędź to wszystko, co kiedykolwiek utworzono.',
  'There are no issues to count yet': 'Nie ma jeszcze zadań do policzenia',
  'in total': 'razem',

  // created vs resolved
  'Issues created and issues resolved, added up from the first day shown. While the lines drift apart, more work arrives than gets finished.':
    'Zadania utworzone i rozwiązane, sumowane od pierwszego pokazanego dnia. Gdy linie się rozchodzą, pracy przybywa szybciej, niż jest kończona.',
  'Nothing was created or resolved in this period': 'W tym okresie niczego nie utworzono ani nie rozwiązano',
  Created: 'Utworzone',
  Resolved: 'Rozwiązane',
  'Open work changed by': 'Zmiana otwartej pracy',
  'created that day': 'utworzone tego dnia',
  'resolved that day': 'rozwiązane tego dnia',
  'Created in total': 'Utworzone łącznie',
  'Resolved in total': 'Rozwiązane łącznie',

  // cycle time
  'Each dot is a resolved issue; the higher it sits, the longer it took. 85 of 100 issues were finished within the p85 line, which makes it a fair promise for the next one.':
    'Każda kropka to rozwiązane zadanie; im wyżej leży, tym dłużej trwało. 85 na 100 zadań zmieściło się pod linią p85, więc to uczciwa obietnica dla następnego.',
  'No issue was resolved in this period': 'W tym okresie nie rozwiązano żadnego zadania',
  '{days} d': '{days} d',
  'half of the issues': 'połowa zadań',
  '{count} never in progress': '{count} bez etapu „w toku”',
  'Days from creation to resolution': 'Dni od utworzenia do rozwiązania',
  'Days from the start of work to resolution': 'Dni od rozpoczęcia pracy do rozwiązania',
  'resolved {when}': 'rozwiązane {when}',
  'from creation': 'od utworzenia',
  'from start of work': 'od rozpoczęcia pracy',
  Issue: 'Zadanie',
  Summary: 'Tytuł',

  // workload
  'Open issues per person right now. The solid part of a bar is already in progress, the pale part has not been started.':
    'Otwarte zadania każdej osoby w tej chwili. Pełna część paska jest już w toku, blada jeszcze nie została rozpoczęta.',
  'There is no open work': 'Nie ma otwartej pracy',
  'Not started': 'Nierozpoczęte',
  'Open issues': 'Otwarte zadania',
  Unassigned: 'Nieprzypisane',
};
