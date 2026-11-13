// The place the local interface leaves for the server's web interface. The app lays that
// interface over the window (`src-tauri/src/stage.rs`); a view only says where its place is.
import { useLayoutEffect, useState, type RefObject } from 'react';

import { inApp, web } from '../bridge.ts';

/**
 * Keeps the server's web interface over `slot` for as long as the calling view is shown
 * and `shown` holds, following the place when it moves or changes size. Returns why it
 * could not be shown, when it could not. `server` is the address it is the interface of:
 * another server, another interface.
 */
export function useWebSlot(slot: RefObject<HTMLElement | null>, server: string | null, shown = true): string | undefined {
  const [failure, setFailure] = useState<string | undefined>();

  useLayoutEffect(() => {
    const element = slot.current;

    if (!inApp || !element || !shown) {
      return undefined;
    }

    let frame = 0;
    let gone = false;
    // Where the place is, in the window's own units, each time it moves or changes size.
    const place = () => {
      window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(() => {
        const box = element.getBoundingClientRect();

        // `page`: how high this page is. On macOS the window measures from above its title bar.
        web.show({ x: box.left, y: box.top, width: box.width, height: box.height, page: window.innerHeight }).then(
          () => !gone && setFailure(undefined),
          (reason: unknown) => !gone && setFailure(reason instanceof Error ? reason.message : String(reason)),
        );
      });
    };
    const observer = new ResizeObserver(place);

    observer.observe(element);
    window.addEventListener('resize', place);
    place();

    return () => {
      gone = true;
      observer.disconnect();
      window.removeEventListener('resize', place);
      window.cancelAnimationFrame(frame);
      void web.hide();
    };
  }, [slot, server, shown]);

  return failure;
}
