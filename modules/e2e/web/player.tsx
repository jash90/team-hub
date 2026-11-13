import { useEffect, useMemo, useRef, useState, type RefObject } from 'react';

import type { E2eSessionArtifact } from '@/bindings/E2eSessionArtifact.ts';
import type { E2eSessionAttempt } from '@/bindings/E2eSessionAttempt.ts';
import type { E2eSessionStep } from '@/bindings/E2eSessionStep.ts';
import { Icon } from '@/components/icon.tsx';
import { t } from '@/i18n/index.ts';

import { ExpandIcon, PauseIcon } from './icons.tsx';
import { artifactUrl } from './shared.tsx';

const SPEEDS = [0.5, 1, 1.5, 2, 4];
/** A step counts as showing from a moment before its start: a seek lands a hair off. */
const SLACK_SECONDS = 0.002;

/** A recording of an attempt and when it began, so steps can be found in it. */
export type Recording = {
  artifact: E2eSessionArtifact;
  /** Where its bytes are; none while the runner has not sent the file. */
  url: string | null;
  /** When it began, in milliseconds since the epoch; NaN when the report does not say. */
  startsAt: number;
};

/** The attempt's recordings, in the order they began. */
export function recordingsOf(attempt: E2eSessionAttempt): Recording[] {
  return attempt.artifacts
    .filter((artifact) => artifact.kind === 'video')
    .map((artifact) => ({
      artifact,
      url: artifact.artifactId !== null ? artifactUrl(artifact.artifactId) : artifact.url,
      // A recording that does not say when it began is taken to cover the attempt.
      startsAt: Date.parse(artifact.startedAt ?? attempt.startedAt ?? ''),
    }))
    .sort((a, b) => a.startsAt - b.startsAt);
}

/**
 * Where a step is in a recording, in seconds: `step.startedAt − recording.startedAt`. A step
 * from before the recording began is at its start; none when either time is unknown.
 */
export function offsetIn(step: E2eSessionStep, recording: Recording): number | null {
  const at = Date.parse(step.startedAt ?? '');

  return Number.isNaN(at) || Number.isNaN(recording.startsAt) ? null : Math.max(0, (at - recording.startsAt) / 1000);
}

/** The recording a step is best watched in: the one playing if it had begun by then, else the last that had. */
export function recordingFor(step: E2eSessionStep, recordings: Recording[], current: number): number {
  const at = Date.parse(step.startedAt ?? '');
  const began = (recording: Recording | undefined) => recording !== undefined && recording.url !== null && !(recording.startsAt > at);

  if (began(recordings[current])) {
    return current;
  }

  const last = recordings.findLastIndex((recording) => began(recording));

  return last === -1 ? current : last;
}

/** The step a recording shows at `seconds`: the last one that had started by then. */
export function stepAt(steps: E2eSessionStep[], recording: Recording, seconds: number): E2eSessionStep | null {
  return steps.findLast((step) => (offsetIn(step, recording) ?? Infinity) <= seconds + SLACK_SECONDS) ?? null;
}

/** A position in a recording to the tenth of a second (steps are often under one apart): `1:04.2`. */
export function clock(seconds: number): string {
  const tenths = Math.max(0, Math.floor(seconds * 10));

  return `${Math.floor(tenths / 600)}:${((tenths % 600) / 10).toFixed(1).padStart(4, '0')}`;
}

/** How long a recording is. One without a length in its header says Infinity; what can be seeked to still ends where the file does. */
export function lengthOf(video: HTMLVideoElement): number {
  if (Number.isFinite(video.duration)) {
    return video.duration;
  }

  return video.seekable.length > 0 ? video.seekable.end(video.seekable.length - 1) : 0;
}

/**
 * A recording with the application's own controls: play, a timeline marked where each step
 * starts, the clock, the speed, the other recordings of the attempt, full screen. The caller
 * keeps `videoRef` and seeks it from the step list; `onTime` tells it where the playhead is.
 */
export function Player({
  recordings,
  index,
  onIndex,
  videoRef,
  steps,
  current,
  onTime,
  onStep,
}: {
  recordings: Recording[];
  index: number;
  onIndex: (index: number) => void;
  videoRef: RefObject<HTMLVideoElement | null>;
  steps: E2eSessionStep[];
  current: E2eSessionStep | null;
  onTime: (seconds: number) => void;
  onStep: (step: E2eSessionStep) => void;
}) {
  const frame = useRef<HTMLDivElement>(null);
  const [paused, setPaused] = useState(true);
  const [time, setTime] = useState(0);
  const [length, setLength] = useState(0);
  const [speed, setSpeed] = useState(1);
  const [broken, setBroken] = useState(false);
  const recording = recordings[index];
  const url = recording?.url ?? null;

  const follow = useRef(onTime);
  follow.current = onTime;

  // Another recording starts from its beginning, at the speed that was chosen.
  useEffect(() => {
    setTime(0);
    setLength(0);
    setBroken(false);
    setPaused(true);
  }, [url]);

  // While it plays, the playhead is read on every frame: the browser's own time updates come
  // a few times a second, and steps are often closer together than that.
  useEffect(() => {
    if (paused) {
      return;
    }

    let frame = requestAnimationFrame(function read() {
      const video = videoRef.current;

      if (video) {
        setTime(video.currentTime);
        follow.current(video.currentTime);
      }

      frame = requestAnimationFrame(read);
    });

    return () => cancelAnimationFrame(frame);
  }, [paused, videoRef]);

  const markers = useMemo(
    () =>
      steps.flatMap((step) => {
        const seconds = recording ? offsetIn(step, recording) : null;

        return seconds === null || (length > 0 && seconds > length) ? [] : [{ step, seconds }];
      }),
    [steps, recording, length],
  );

  if (!recording || url === null) {
    return null;
  }

  const percent = (seconds: number) => `${length > 0 ? Math.min(100, (seconds / length) * 100) : 0}%`;

  function toggle(): void {
    const video = videoRef.current;

    if (video) {
      void (video.paused ? video.play() : video.pause());
    }
  }

  function loaded(video: HTMLVideoElement): void {
    setLength(lengthOf(video));
    video.playbackRate = speed;
  }

  function tick(video: HTMLVideoElement): void {
    setTime(video.currentTime);
    onTime(video.currentTime);
  }

  return (
    <div className="e2e-player" ref={frame}>
      {broken ? (
        <div className="e2e-stage-empty">
          <strong>{t('This recording cannot be played here')}</strong>
          <span className="muted">{t('The browser does not play this kind of video, or the file is damaged.')}</span>
          <a href={url} download>
            {t('Download the recording')}
          </a>
        </div>
      ) : (
        <video
          ref={videoRef}
          key={url}
          src={url}
          className="e2e-video"
          muted
          playsInline
          preload="auto"
          aria-label={t('Recording of the test')}
          onClick={toggle}
          onPlay={() => setPaused(false)}
          onPause={() => setPaused(true)}
          onLoadedMetadata={(event) => loaded(event.currentTarget)}
          onDurationChange={(event) => loaded(event.currentTarget)}
          onTimeUpdate={(event) => tick(event.currentTarget)}
          onSeeked={(event) => tick(event.currentTarget)}
          onError={() => setBroken(true)}
        />
      )}
      <div className="e2e-player-bar">
        <div className="e2e-timeline">
          <div className="e2e-timeline-track" />
          <div className="e2e-timeline-played" style={{ width: percent(time) }} />
          {markers.map(({ step, seconds }) => (
            <span
              key={step.index}
              className={`e2e-marker${step.index === current?.index ? ' e2e-marker-now' : ''}${step.status === 'failed' ? ' e2e-marker-failed' : ''}`}
              style={{ left: percent(seconds) }}
            />
          ))}
          <input
            type="range"
            className="e2e-seek"
            aria-label={t('Position in the recording')}
            min={0}
            max={length || 0}
            step={0.05}
            value={time}
            disabled={broken}
            onChange={(event) => {
              if (videoRef.current) {
                videoRef.current.currentTime = Number(event.currentTarget.value);
              }
            }}
          />
        </div>
        <div className="e2e-player-controls">
          <button type="button" className="e2e-player-button" aria-label={paused ? t('Play') : t('Pause')} title={`${paused ? t('Play') : t('Pause')} (${t('space')})`} disabled={broken} onClick={toggle}>
            {paused ? <Icon name="play" size={15} /> : <PauseIcon size={15} />}
          </button>
          <span className="e2e-clock" aria-label={t('Position in the recording')}>
            {clock(time)} / {clock(length)}
          </span>
          <span className="e2e-now" title={current ? `${current.api} ${current.label}` : undefined}>
            {current && (
              <button type="button" className="e2e-now-step" onClick={() => onStep(current)}>
                <span className="mono">{current.api}</span> {current.label}
              </button>
            )}
          </span>
          {recordings.length > 1 && (
            <span className="e2e-parts" role="group" aria-label={t('Recordings of this attempt')}>
              {recordings.map((part, partIndex) => (
                <button
                  key={part.artifact.ref}
                  type="button"
                  className="e2e-player-button"
                  aria-pressed={partIndex === index}
                  disabled={part.url === null}
                  title={part.url === null ? t('The runner has not sent this file') : t('Recording {number} of {count}', { number: partIndex + 1, count: recordings.length })}
                  onClick={() => onIndex(partIndex)}
                >
                  {partIndex + 1}
                </button>
              ))}
            </span>
          )}
          <select
            className="e2e-speed"
            aria-label={t('Playback speed')}
            title={t('Playback speed')}
            value={speed}
            onChange={(event) => {
              const next = Number(event.currentTarget.value);

              setSpeed(next);

              if (videoRef.current) {
                videoRef.current.playbackRate = next;
              }
            }}
          >
            {SPEEDS.map((option) => (
              <option key={option} value={option}>
                {option}×
              </option>
            ))}
          </select>
          <a className="e2e-player-button" href={url} download aria-label={t('Download the recording')} title={t('Download the recording')}>
            <Icon name="download" size={15} />
          </a>
          <button
            type="button"
            className="e2e-player-button"
            aria-label={t('Full screen')}
            title={t('Full screen')}
            onClick={() => void (document.fullscreenElement ? document.exitFullscreen() : frame.current?.requestFullscreen())}
          >
            <ExpandIcon size={15} />
          </button>
        </div>
      </div>
    </div>
  );
}
