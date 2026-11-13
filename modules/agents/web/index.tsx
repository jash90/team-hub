import type { WebModule } from '@/modules.ts';

import { polish } from './pl.ts';

export const agentsModule: WebModule = {
  id: 'agents',
  polish,
};
