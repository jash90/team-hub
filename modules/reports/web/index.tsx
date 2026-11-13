import type { WebModule } from '@/modules.ts';

import { polish } from './pl.ts';

export const reportsModule: WebModule = {
  id: 'reports',
  polish,
};
