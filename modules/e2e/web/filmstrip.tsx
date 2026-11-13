import { useEffect, useRef, useState } from 'react';

import type { E2eSessionStep } from '@/bindings/E2eSessionStep.ts';
import { t } from '@/i18n/index.ts';

import { clock, lengthOf } from './player.tsx';

/** A recording of a long test is not cut into more pictures than a strip can show. */
const MAX_FRAMES = 60;
const FRAME_WIDTH = 240;
/** How long a hidden video is given to load or to seek before the strip gives up on it. */
const PATIENCE_MS = 15_000;

export type Moment = { step: E2eSessionStep; seconds: number };

/** At most `MAX_FRAMES` of the moments, evenly spread, the first and the failed ones always among them. */
export function sampled(moments: Moment[]): Moment[] {
  if (moments.length <= MAX_FRAMES) {
    return moments;
  }

  const every = moments.length / MAX_FRAMES;
  const kept = new Set(Array.from({ length: MAX_FRAMES }, (_, index) => Math.floor(index * every)));

  return moments.filter((moment, index) => kept.has(index) || moment.step.status === 'failed');
}

function waitFor(video: HTMLVideoElement, event: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const timer = window.setTimeout(() => done(() => reject(new Error(`no ${event}`))), PATIENCE_MS);
    const failed = () => done(() => reject(new Error('the video cannot be read')));
    const arrived = () => done(resolve);

    function done(settle: () => void): void {
      window.clearTimeout(timer);
      video.removeEventListener(event, arrived);
      video.removeEventListener('error', failed);
      settle();
    }

    video.addEventListener(event, arrived);
    video.addEventListener('error', failed);
  });
}

/**
 * A picture of the recording at each moment, made here in the browser: a second, hidden
 * video is seeked to the moment and drawn onto a canvas. The pictures arrive one by one, and
 * the work stops when the strip goes away or another recording is chosen. Keyed by step index.
 */
function useFrames(url: string, moments: Moment[], wanted: boolean): Record<number, string> {
  const [frames, setFrames] = useState<Record<number, string>>({});
  // The moments as text: the list is built anew on every render, its content rarely changes.
  const plan = moments.map((moment) => `${moment.step.index}@${moment.seconds.toFixed(2)}`).join(' ');
  const latest = useRef(moments);
  latest.current = moments;

  useEffect(() => {
    setFrames({});

    if (!wanted || latest.current.length === 0) {
      return;
    }

    let cancelled = false;
    const video = document.createElement('video');
    const canvas = document.createElement('canvas');

    async function draw(): Promise<void> {
      video.muted = true;
      video.preload = 'auto';
      video.src = url;
      await waitFor(video, 'loadeddata');

      const context = canvas.getContext('2d');
      const end = lengthOf(video);

      if (!context || video.videoWidth === 0) {
        return;
      }

      canvas.width = FRAME_WIDTH;
      canvas.height = Math.round((FRAME_WIDTH * video.videoHeight) / video.videoWidth);

      for (const moment of latest.current) {
        if (cancelled) {
          return;
        }

        // The very end of a file has no frame to show; a moment before it does.
        video.currentTime = Math.max(0, Math.min(moment.seconds, end > 0 ? end - 0.05 : moment.seconds));
        await waitFor(video, 'seeked');

        if (cancelled) {
          return;
        }

        context.drawImage(video, 0, 0, canvas.width, canvas.height);
        const picture = canvas.toDataURL('image/jpeg', 0.72);
        setFrames((before) => ({ ...before, [moment.step.index]: picture }));
        // Let the page breathe between pictures: the visible video is playing.
        await new Promise((resolve) => window.setTimeout(resolve, 0));
      }
    }

    // A recording that cannot be read leaves the strip with its numbers and no pictures.
    draw().catch(() => undefined);

    return () => {
      cancelled = true;
      video.removeAttribute('src');
      video.load();
    };
  }, [url, plan, wanted]);

  return frames;
}

/**
 * The recording as a strip of pictures, one per step, so that single moments can be looked at
 * and jumped to although the framework only photographs failures.
 */
export function Filmstrip({ url, moments, current, onStep }: { url: string; moments: Moment[]; current: E2eSessionStep | null; onStep: (step: E2eSessionStep) => void }) {
  const strip = useRef<HTMLDivElement>(null);
  // Pictures are made once the strip has been on screen: a page opened for its error alone pays nothing.
  const [seen, setSeen] = useState(false);
  const shown = sampled(moments);
  const frames = useFrames(url, shown, seen);
  const made = Object.keys(frames).length;

  useEffect(() => {
    const element = strip.current;

    if (!element || seen) {
      return;
    }

    // "On screen" starts a little before it: the pictures are there when the reader arrives.
    const observer = new IntersectionObserver((entries) => entries.some((entry) => entry.isIntersecting) && setSeen(true), { rootMargin: '300px' });
    observer.observe(element);

    return () => observer.disconnect();
  }, [seen]);

  // The picture of the step being shown stays in view as the recording plays.
  useEffect(() => {
    const element = strip.current;
    const tile = element?.querySelector<HTMLElement>('[aria-current="true"]');

    if (element && tile) {
      element.scrollTo({ left: tile.offsetLeft - element.clientWidth / 2 + tile.clientWidth / 2, behavior: 'smooth' });
    }
  }, [current?.index]);

  if (shown.length === 0) {
    return null;
  }

  return (
    <section className="e2e-strip-box" aria-label={t('The recording, step by step')}>
      <header className="e2e-strip-head">
        <strong>{t('Step by step')}</strong>
        <span className="muted" aria-live="polite">
          {made < shown.length ? t('{made} of {count} pictures', { made, count: shown.length }) : t('{count} pictures from the recording', { count: shown.length })}
        </span>
      </header>
      <div className="e2e-strip" ref={strip}>
        {shown.map(({ step, seconds }) => (
          <button
            key={step.index}
            type="button"
            className={`e2e-frame${step.status === 'failed' ? ' e2e-frame-failed' : ''}`}
            aria-current={step.index === current?.index}
            title={`${step.api} ${step.label}`}
            onClick={() => onStep(step)}
          >
            {frames[step.index] ? <img src={frames[step.index]} alt={t('Step {number} at {time}', { number: step.index + 1, time: clock(seconds) })} /> : <span className="e2e-frame-wait" />}
            <span className="e2e-frame-label">
              <span>{step.index + 1}</span>
              <span className="mono">{step.api}</span>
              <span className="muted">{clock(seconds)}</span>
            </span>
          </button>
        ))}
      </div>
    </section>
  );
}
