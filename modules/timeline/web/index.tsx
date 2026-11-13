import type { WebModule } from '@/modules.ts';

import { polish } from './pl.ts';
import './styles.css';

const timelineModule: WebModule = {
  id: 'timeline',
  polish,
};

export default timelineModule;
