import { useState, type DragEvent } from 'react';

/** Where a dragged issue would land: in a zone (a column, a sprint), between two neighbours. */
export type DropSpot = { zone: string; beforeKey: string | null; afterKey: string | null };

let draggedKey: string | null = null;

export function draggedIssue(): string | null {
  return draggedKey;
}

function spotAt(container: HTMLElement, zone: string, y: number): DropSpot {
  const items = Array.from(container.querySelectorAll<HTMLElement>('[data-key]')).filter((item) => item.dataset.key !== draggedKey);
  let index = items.findIndex((item) => {
    const box = item.getBoundingClientRect();

    return y < box.top + box.height / 2;
  });

  if (index === -1) {
    index = items.length;
  }

  return { zone, beforeKey: items[index]?.dataset.key ?? null, afterKey: items[index - 1]?.dataset.key ?? null };
}

/**
 * Drag and drop of issues between and within zones. `onDrop` gets the dragged key and the
 * spot; the hook only tracks where the pointer is, for the insertion line.
 */
export function useIssueDrag(onDrop: (key: string, spot: DropSpot) => void) {
  const [spot, setSpot] = useState<DropSpot | null>(null);
  const [dragging, setDragging] = useState<string | null>(null);

  return {
    spot,
    dragging,
    /** Props for a draggable issue element. */
    item: (key: string, enabled = true) => ({
      'data-key': key,
      draggable: enabled,
      onDragStart: (event: DragEvent<HTMLElement>) => {
        draggedKey = key;
        setDragging(key);
        event.dataTransfer.effectAllowed = 'move';
        event.dataTransfer.setData('text/plain', key);
      },
      onDragEnd: () => {
        draggedKey = null;
        setDragging(null);
        setSpot(null);
      },
    }),
    /** Props for a container issues can be dropped into. */
    zone: (zone: string) => ({
      onDragOver: (event: DragEvent<HTMLElement>) => {
        if (!draggedKey) {
          return;
        }

        event.preventDefault();
        event.dataTransfer.dropEffect = 'move';
        const next = spotAt(event.currentTarget, zone, event.clientY);

        setSpot((current) => (current && current.zone === next.zone && current.beforeKey === next.beforeKey ? current : next));
      },
      onDrop: (event: DragEvent<HTMLElement>) => {
        event.preventDefault();
        const key = draggedKey;
        const target = spotAt(event.currentTarget, zone, event.clientY);

        draggedKey = null;
        setDragging(null);
        setSpot(null);

        if (key) {
          onDrop(key, target);
        }
      },
    }),
    /** True when the insertion line belongs right before `key` (or at the end of the zone for null). */
    lineBefore: (zone: string, key: string | null) => spot !== null && spot.zone === zone && spot.beforeKey === key,
  };
}
