// Polish for the desktop app. English is written in place (`t('…')`), exactly as in the web
// interface, whose own dictionary answers first: words the two share are translated once,
// there. The modules' dictionaries cover what their panels say.
import { addTranslations } from '@web/i18n/index.ts';
import { polish as agents } from '@modules/agents/web/pl.ts';
import { polish as github } from '@modules/github/web/pl.ts';
import { polish } from './pl.ts';

addTranslations(github);
addTranslations(agents);
addTranslations(polish);
