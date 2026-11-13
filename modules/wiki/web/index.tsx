import type { WebModule } from '@/modules.ts';

import { polish } from './pl.ts';

export const wikiModule: WebModule = {
  id: 'wiki',
  polish,
};
