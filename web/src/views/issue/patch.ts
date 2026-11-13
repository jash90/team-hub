import type { IssuePatch } from '../../bindings/IssuePatch.ts';

/** The fields of an issue that one change touches. */
export type Patch = Partial<IssuePatch>;
