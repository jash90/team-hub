import type { WebModule } from '@/modules.ts';

import { polish } from './pl.ts';

export const e2eModule: WebModule = {
  id: 'e2e',
  polish,
};
