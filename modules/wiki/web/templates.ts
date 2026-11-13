import type { WikiTemplate } from '@/bindings/WikiTemplate.ts';
import { currentLanguage } from '@/i18n/index.ts';

/** The built-in templates in Polish, by template id; the server holds the English ones. */
const POLISH: Record<string, string> = {
  'decision-record':
    '**Status:** proponowana | przyjęta | zastąpiona\n**Data:** RRRR-MM-DD\n**Decydujący:** \n\n## Kontekst\n\nCo sprawia, że decyzja jest potrzebna właśnie teraz? Jakie są ograniczenia?\n\n## Rozważane opcje\n\n1. **Opcja A**: \n2. **Opcja B**: \n\n## Decyzja\n\nRobimy …\n\n## Skutki\n\n- Co staje się łatwiejsze:\n- Co staje się trudniejsze:\n\n## Powiązane zgłoszenia\n\n- \n',
  'meeting-notes': '**Data:** RRRR-MM-DD\n**Obecni:** \n\n## Plan\n\n1. \n\n## Notatki\n\n- \n\n## Ustalenia\n\n- \n\n## Zadania\n\n- [ ] \n',
  runbook:
    '## Kiedy używać\n\nSytuacja, której dotyczy ta procedura, i kto może ją wykonać.\n\n## Zanim zaczniesz\n\n- [ ] Dostęp do …\n- [ ] Zespół wie o tym z …\n\n## Kroki\n\n1. \n2. \n3. \n\n## Sprawdzenie\n\nPo czym poznać, że się udało.\n\n## Wycofanie\n\nJak cofnąć każdy krok, jeśli się nie udało.\n\n## Kontakty\n\n- \n',
  'technical-spec':
    '## Streszczenie\n\nJeden akapit: co budujemy i po co.\n\n## Cele\n\n- \n\n## Poza zakresem\n\n- \n\n## Projekt\n\n### Model danych\n\n### API\n\n### Co może pójść źle\n\n## Rozważane alternatywy\n\n## Wdrożenie\n\n- [ ] \n\n## Otwarte pytania\n\n- \n',
  retrospective: '**Sprint:** \n**Data:** RRRR-MM-DD\n\n## Co poszło dobrze\n\n- \n\n## Co poszło gorzej\n\n- \n\n## Czego się nauczyliśmy\n\n- \n\n## Ustalenia\n\n- [ ] \n',
};

/** The body a new page starts with, in the language of the interface. */
export function templateBody(template: WikiTemplate): string {
  return (currentLanguage() === 'pl' ? POLISH[template.id] : undefined) ?? template.body;
}
