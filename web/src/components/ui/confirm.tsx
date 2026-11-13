import type { ReactNode } from 'react';

import { t } from '../../i18n/index.ts';
import { Button } from './button.tsx';
import { Dialog } from './dialog.tsx';
import { useBusy } from './toast.tsx';

export function ConfirmDialog({ title, body, confirm, danger, onConfirm, onClose }: { title: string; body: ReactNode; confirm: string; danger?: boolean; onConfirm: () => void | Promise<void>; onClose: () => void }) {
  const [busy, run] = useBusy();

  return (
    <Dialog
      title={title}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{t('Cancel')}</Button>
          <Button
            variant={danger ? 'danger' : 'primary'}
            disabled={busy}
            onClick={() =>
              void run(async () => {
                await onConfirm();
                onClose();
              })
            }
          >
            {confirm}
          </Button>
        </>
      }
    >
      <div className="dialog-text">{body}</div>
    </Dialog>
  );
}
