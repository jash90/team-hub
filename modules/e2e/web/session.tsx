import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';

import { api } from '@/api.ts';
import type { E2eSession } from '@/bindings/E2eSession.ts';
import type { E2eSessionArtifact } from '@/bindings/E2eSessionArtifact.ts';
import type { E2eSessionAttempt } from '@/bindings/E2eSessionAttempt.ts';
import type { E2eSessionError } from '@/bindings/E2eSessionError.ts';
import type { E2eSessionLogLine } from '@/bindings/E2eSessionLogLine.ts';
import type { E2eSessionStep } from '@/bindings/E2eSessionStep.ts';
import { Icon } from '@/components/icon.tsx';
import { Markdown } from '@/components/markdown.tsx';
import { Button, Dialog, Empty, Spinner, Tabs, Tag } from '@/components/ui.tsx';
import { formatDateTime, t } from '@/i18n/index.ts';
import { Link, setParam, useSearchParams } from '@/router.tsx';
import { useProject, useRemote } from '@/session.tsx';

import { Actions, failureOfResult, type Acting } from './actions.tsx';
import { Filmstrip, type Moment } from './filmstrip.tsx';
import { BackIcon, CameraIcon, VideoIcon } from './icons.tsx';
import { Player, clock, offsetIn, recordingFor, recordingsOf, stepAt } from './player.tsx';
import { BASE, IssueChips, Origin, StatusMark, artifactUrl, duration, jobPath, runPath } from './shared.tsx';

/** How an attempt ended, said of the attempt (Polish needs its own forms: "próba" is feminine). */
const ATTEMPT_TEXT: Record<string, string> = {
  passed: 'Attempt {number} · passed',
  failed: 'Attempt {number} · failed',
  'timed-out': 'Attempt {number} · timed out',
  interrupted: 'Attempt {number} · interrupted',
  skipped: 'Attempt {number} · skipped',
};
const LOG_SHOWN = 40;

function nameOf(artifact: E2eSessionArtifact): string {
  return artifact.path?.split('/').pop() ?? artifact.ref;
}

function size(bytes: number | null): string {
  if (bytes === null) {
    return '';
  }

  return bytes < 1024 * 1024 ? `${Math.max(1, Math.round(bytes / 1024))} kB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/** A file of the run as text, for the ones meant to be read: a trace page, the screen at failure. */
function useText(artifactId: number | null, wanted: boolean): { text: string | null; error: string | null } {
  const remote = useRemote(
    async () => {
      if (artifactId === null || !wanted) {
        return null;
      }

      const body = await api.get<unknown>(artifactUrl(artifactId));

      return typeof body === 'string' ? body : JSON.stringify(body, null, 2);
    },
    [artifactId, wanted],
  );

  return { text: remote.data ?? null, error: remote.error };
}

// ---------- an error ----------

/** An error the way it reads best: its code and where it was thrown, its first line, then what was expected and what was there. */
function ErrorBlock({ error, also }: { error: E2eSessionError; also?: boolean }) {
  const [headline = '', ...rest] = error.message.split('\n');
  const restated = /^(locator|expected|observed): /;
  const facts: [string, string, string][] = [];

  if (error.locator) {
    facts.push([t('locator'), error.locator, '']);
  }

  if (error.expected) {
    facts.push([t('expected'), error.expected, 'e2e-fact-expected']);
  }

  if (error.observed) {
    facts.push([t('received'), error.observed, 'e2e-fact-received']);
  }

  // The message repeats the facts as lines; they are shown once, as the table.
  const more = rest.filter((line) => line.trim() !== '' && !(facts.length > 0 && restated.test(line)));

  return (
    <section className={also ? 'e2e-error e2e-error-also' : 'e2e-error'} aria-label={also ? t('Also failed') : t('Error')}>
      <span className="e2e-error-head">
        {error.code && <strong className="mono">{error.code}</strong>}
        {error.file && (
          <span className="mono muted">
            {error.file}
            {error.line ? `:${error.line}` : ''}
          </span>
        )}
        {also && <span className="muted">{t('also failed')}</span>}
      </span>
      <p className="e2e-error-line">{headline || error.code}</p>
      {facts.length > 0 && (
        <dl className="e2e-error-facts">
          {facts.map(([label, value, tone]) => (
            <div key={label}>
              <dt>{label}</dt>
              <dd className={tone}>{value}</dd>
            </div>
          ))}
        </dl>
      )}
      {more.length > 0 && <pre className="e2e-error-more">{more.join('\n')}</pre>}
    </section>
  );
}

// ---------- screenshots ----------

/** A screenshot the report names, with what it shows: the failure, or the step that took it. */
type Shot = { artifact: E2eSessionArtifact; label: string; step: E2eSessionStep | null };

function shotsOf(attempt: E2eSessionAttempt): Shot[] {
  return attempt.artifacts
    .filter((artifact) => artifact.kind === 'screenshot')
    .map((artifact) => {
      const step = attempt.steps.find((candidate) => candidate.artifacts.includes(artifact.ref)) ?? null;
      const label = artifact.ref === attempt.failure?.screenshot ? t('At failure') : step ? `${step.api} ${step.label}` : nameOf(artifact);

      return { artifact, label, step };
    });
}

function NotSent({ active }: { active: boolean }) {
  return (
    <span className="e2e-not-sent">
      <Icon name="clock" size={14} />
      {t('The runner has not sent this file')}
      {active && <span className="muted">{t('The job is still at work; it may yet arrive.')}</span>}
    </span>
  );
}

function Enlarged({ shot, onClose }: { shot: Shot; onClose: () => void }) {
  const url = shot.artifact.artifactId !== null ? artifactUrl(shot.artifact.artifactId) : null;

  return (
    <Dialog title={shot.label} wide onClose={onClose}>
      {url && (
        <a href={url} target="_blank" rel="noreferrer" title={t('Open the picture in a new tab')}>
          <img className="e2e-enlarged" src={url} alt={shot.label} />
        </a>
      )}
    </Dialog>
  );
}

/** One screenshot as a tile: the picture when the runner has sent it, and its label. */
function ShotTile({ shot, active, selected, onSelect }: { shot: Shot; active: boolean; selected?: boolean; onSelect: () => void }) {
  const id = shot.artifact.artifactId;

  return (
    <button type="button" className="e2e-shot" aria-current={selected} disabled={id === null} title={shot.label} onClick={onSelect}>
      {id !== null ? <img src={artifactUrl(id)} alt={shot.label} loading="lazy" /> : <NotSent active={active} />}
      <span className="e2e-shot-label">{shot.label}</span>
    </button>
  );
}

/**
 * What an attempt without a recording has to look at: the screenshots its report names, one
 * large and all of them as a strip. Without any, it says why there is nothing.
 */
function Stills({ attempt, shots, chosen, onChoose, active }: { attempt: E2eSessionAttempt; shots: Shot[]; chosen: Shot | null; onChoose: (shot: Shot) => void; active: boolean }) {
  const [enlarged, setEnlarged] = useState(false);
  const named = attempt.artifacts.some((artifact) => artifact.kind === 'video');

  if (!chosen) {
    return (
      <div className="e2e-stage-empty">
        <VideoIcon size={26} />
        {named ? (
          <>
            <strong>{t('The recording has not arrived')}</strong>
            <NotSent active={active} />
          </>
        ) : (
          <>
            <strong>{t('This attempt was not recorded')}</strong>
            <span className="muted">{t('Turn video on in the project’s test settings, or when starting a job, to watch a test afterwards. A failed attempt still gets a screenshot.')}</span>
          </>
        )}
      </div>
    );
  }

  const id = chosen.artifact.artifactId;

  return (
    <div className="e2e-stills">
      <figure className="e2e-still">
        {id !== null ? (
          <button type="button" className="e2e-still-picture" title={t('Enlarge')} onClick={() => setEnlarged(true)}>
            <img src={artifactUrl(id)} alt={chosen.label} />
          </button>
        ) : (
          <div className="e2e-stage-empty">
            <NotSent active={active} />
          </div>
        )}
        <figcaption>
          <CameraIcon size={14} />
          <span>{chosen.label}</span>
          {named ? <span className="muted">{t('The recording has not arrived; these are the screenshots.')}</span> : <span className="muted">{t('No recording; these are the screenshots the test took.')}</span>}
        </figcaption>
      </figure>
      <div className="e2e-strip e2e-strip-stills" aria-label={t('Screenshots')}>
        {shots.map((shot) => (
          <ShotTile key={shot.artifact.ref} shot={shot} active={active} selected={shot.artifact.ref === chosen.artifact.ref} onSelect={() => onChoose(shot)} />
        ))}
      </div>
      {enlarged && <Enlarged shot={chosen} onClose={() => setEnlarged(false)} />}
    </div>
  );
}

// ---------- steps ----------

const EVENT_HEAD: Record<string, string> = { engine: 'Actions', navigation: 'Navigation', poll: 'What it read' };

function StepDetails({ step, attempt, shots, onShot }: { step: E2eSessionStep; attempt: E2eSessionAttempt; shots: Shot[]; onShot: (shot: Shot) => void }) {
  // The step that failed the attempt carries the attempt's error, already shown above the steps.
  const error = step.error && step.error.message !== attempt.error?.message ? step.error : null;
  const logged = attempt.appLog.filter((line) => line.step === step.index);
  const taken = shots.filter((shot) => shot.step?.index === step.index);

  return (
    <div className="e2e-step-details">
      {error && <ErrorBlock error={error} />}
      {step.explanation && <p className="e2e-step-said">{step.explanation}</p>}
      {(['engine', 'navigation', 'poll'] as const).map((kind) => {
        const events = step.events.filter((event) => event.kind === kind);

        return (
          events.length > 0 && (
            <div key={kind} className="e2e-step-part">
              <span className="e2e-step-part-head">{t(EVENT_HEAD[kind])}</span>
              <ul>
                {events.map((event, index) => (
                  <li key={index} className={event.failed ? 'e2e-event-failed' : undefined}>
                    <span>{event.detail}</span>
                    {event.durationMs > 0 && <span className="muted"> {duration(event.durationMs)}</span>}
                  </li>
                ))}
              </ul>
            </div>
          )
        );
      })}
      {step.moreEvents > 0 && <span className="muted">{t('… and {count} more', { count: step.moreEvents })}</span>}
      {(step.location || step.changes.length > 0) && (
        <div className="e2e-step-part">
          <span className="e2e-step-part-head">{t('Screen after the step')}</span>
          {step.location && <span className="mono e2e-step-location">{step.location}</span>}
          {step.changes.length > 0 && (
            <pre className="e2e-changes">
              {step.changes.map((line, index) => (
                <span key={index} className={line.startsWith('added') || line.startsWith('+') ? 'e2e-change-added' : line.startsWith('removed') || line.startsWith('-') ? 'e2e-change-removed' : undefined}>
                  {line}
                </span>
              ))}
              {step.moreChanges > 0 && <span className="muted">{t('… and {count} more', { count: step.moreChanges })}</span>}
            </pre>
          )}
        </div>
      )}
      {logged.length > 0 && (
        <div className="e2e-step-part">
          <span className="e2e-step-part-head">{t('App log')}</span>
          <ul>
            {logged.map((line, index) => (
              <li key={index} className={`e2e-log-${line.level}`}>
                {line.text}
              </li>
            ))}
          </ul>
        </div>
      )}
      {taken.length > 0 && (
        <div className="e2e-step-shots">
          {taken.map((shot) => (
            <ShotTile key={shot.artifact.ref} shot={shot} active={false} onSelect={() => onShot(shot)} />
          ))}
        </div>
      )}
    </div>
  );
}

function hasDetails(step: E2eSessionStep, attempt: E2eSessionAttempt): boolean {
  return (
    step.error !== null || step.explanation !== null || step.events.length > 0 || step.changes.length > 0 || step.location !== null || step.artifacts.length > 0 || attempt.appLog.some((line) => line.step === step.index)
  );
}

function StepRow({
  step,
  attempt,
  shots,
  offset,
  current,
  onSelect,
  onShot,
}: {
  step: E2eSessionStep;
  attempt: E2eSessionAttempt;
  shots: Shot[];
  /** Where the step is in the recording being shown, in seconds; none without one. */
  offset: number | null;
  current: boolean;
  onSelect: () => void;
  onShot: (shot: Shot) => void;
}) {
  const [open, setOpen] = useState(step.status === 'failed');
  const details = hasDetails(step, attempt);
  const status = step.status === 'passed' ? 'passed' : step.status === 'failed' ? 'failed' : 'skipped';

  return (
    <li className={`e2e-step e2e-is-${status}`} aria-current={current ? 'step' : undefined} data-step={step.index}>
      <div className="e2e-step-row">
        <button type="button" className="e2e-step-main" title={offset === null ? undefined : t('Show this moment in the recording')} onClick={onSelect}>
          <span className="e2e-step-number">{step.index + 1}</span>
          <StatusMark status={status} label={false} />
          <span className="e2e-step-text">
            <span className="e2e-step-what">
              <span className="mono e2e-step-api">{step.api}</span> <span className="e2e-step-label">{step.label}</span>
            </span>
            <span className="e2e-step-meta muted">
              {step.phase && <span className="e2e-tag">{step.phase}</span>}
              {offset !== null && <span className="e2e-step-clock mono">{clock(offset)}</span>}
              <span>{duration(step.durationMs)}</span>
              {step.line !== null && <span className="mono" title={step.file ?? undefined}>{`${step.file?.split('/').pop() ?? ''}:${step.line}`}</span>}
            </span>
          </span>
        </button>
        {details && (
          <button type="button" className="e2e-step-more" aria-expanded={open} aria-label={open ? t('Hide details') : t('Show details')} title={open ? t('Hide details') : t('Show details')} onClick={() => setOpen(!open)}>
            <Icon name={open ? 'chevronDown' : 'chevronRight'} size={13} />
          </button>
        )}
      </div>
      {open && details && <StepDetails step={step} attempt={attempt} shots={shots} onShot={onShot} />}
    </li>
  );
}

// ---------- what else an attempt left ----------

function Section({ title, count, children, action }: { title: string; count?: number; children: ReactNode; action?: ReactNode }) {
  return (
    <section className="panel e2e-section">
      <header className="panel-head">
        <h2>{title}</h2>
        {count !== undefined && <span className="tab-count">{count}</span>}
        <span className="filter-spacer" />
        {action}
      </header>
      <div className="e2e-section-body">{children}</div>
    </section>
  );
}

/** What the runner saw when the failure landed: the address, the screen as text, what was closest to the locator. */
function AtFailure({ attempt, active }: { attempt: E2eSessionAttempt; active: boolean }) {
  const [open, setOpen] = useState(false);
  const failure = attempt.failure;
  const screen = attempt.artifacts.find((artifact) => artifact.ref === failure?.screen) ?? null;
  const text = useText(screen?.artifactId ?? null, open);

  if (!failure) {
    return null;
  }

  return (
    <Section title={t('At failure')}>
      {failure.url && <span className="mono e2e-step-location">{failure.url}</span>}
      {failure.candidates.length > 0 && (
        <div className="e2e-step-part">
          <span className="e2e-step-part-head">{t('Closest on screen to what was asked for')}</span>
          <pre className="e2e-changes">{failure.candidates.join('\n')}</pre>
        </div>
      )}
      {screen && (
        <div className="e2e-step-part">
          <span className="e2e-step-part-head">
            {t('The screen as text')}
            {failure.nodes !== null && <span className="muted"> · {t('{count} elements', { count: failure.nodes })}</span>}
          </span>
          {screen.artifactId === null ? (
            <NotSent active={active} />
          ) : open ? (
            text.text === null ? (
              text.error ? <span className="muted">{text.error}</span> : <Spinner />
            ) : (
              <pre className="e2e-screen-text">{text.text}</pre>
            )
          ) : (
            <div>
              <Button size="sm" onClick={() => setOpen(true)}>
                {t('Show what was on screen')}
              </Button>
            </div>
          )}
        </div>
      )}
    </Section>
  );
}

const LOUD = ['error', 'warning', 'warn'];

/** What the app logged: errors and warnings first, each with the step it happened in. */
function AppLog({ lines, more, onStep }: { lines: E2eSessionLogLine[]; more: number; onStep: (index: number) => void }) {
  const [all, setAll] = useState(false);
  const ordered = useMemo(() => [...lines.filter((line) => LOUD.includes(line.level)), ...lines.filter((line) => !LOUD.includes(line.level))], [lines]);
  const shown = all ? ordered : ordered.slice(0, LOG_SHOWN);

  if (lines.length === 0) {
    return null;
  }

  return (
    <Section title={t('App log')} count={lines.length + more}>
      <ul className="e2e-applog">
        {shown.map((line, index) => (
          <li key={index} className={`e2e-log-${line.level}`}>
            <span className="e2e-applog-level">{line.level}</span>
            <span className="e2e-applog-text">{line.text}</span>
            {line.step !== null && (
              <button type="button" className="link-button" onClick={() => onStep(line.step ?? 0)}>
                {t('step {number}', { number: (line.step ?? 0) + 1 })}
              </button>
            )}
          </li>
        ))}
      </ul>
      {(ordered.length > shown.length || more > 0) && (
        <div className="e2e-section-foot muted">
          {ordered.length > shown.length && (
            <button type="button" className="link-button" onClick={() => setAll(true)}>
              {t('Show all {count}', { count: ordered.length })}
            </button>
          )}
          {more > 0 && <span>{t('{count} older lines were not kept.', { count: more })}</span>}
        </div>
      )}
    </Section>
  );
}

/** Every file the report names for the attempt, to download, with the ones that have not arrived. */
function Files({ attempt, active }: { attempt: E2eSessionAttempt; active: boolean }) {
  if (attempt.artifacts.length === 0) {
    return null;
  }

  return (
    <Section title={t('Files')} count={attempt.artifacts.length}>
      <ul className="e2e-files-list">
        {attempt.artifacts.map((artifact) => (
          <li key={artifact.ref}>
            <Icon name={artifact.kind === 'video' || artifact.kind === 'screenshot' ? 'eye' : 'page'} size={14} />
            {artifact.artifactId !== null ? (
              <a href={artifactUrl(artifact.artifactId)} download={nameOf(artifact)} title={artifact.path ?? undefined}>
                {nameOf(artifact)}
              </a>
            ) : artifact.url ? (
              <a href={artifact.url} target="_blank" rel="noreferrer">
                {nameOf(artifact)}
              </a>
            ) : (
              <span title={artifact.path ?? undefined}>{nameOf(artifact)}</span>
            )}
            <span className="muted">{size(artifact.size)}</span>
            {artifact.artifactId === null && !artifact.url && <NotSent active={active} />}
          </li>
        ))}
      </ul>
    </Section>
  );
}

/** The test's trace page (`trace.md`): every step with what it did, as the framework wrote it. */
function Trace({ artifactId }: { artifactId: number | null }) {
  const [open, setOpen] = useState(false);
  const page = useText(artifactId, open);

  if (artifactId === null) {
    return <p className="muted e2e-trace-none">{t('No trace page was sent for this test. Trace pages are kept for failed tests unless the project’s test settings say “every test”.')}</p>;
  }

  return (
    <Section
      title={t('Trace page')}
      action={
        <>
          <a className="e2e-more" href={artifactUrl(artifactId)} download="trace.md">
            {t('Download')}
          </a>
          <Button size="sm" icon={open ? 'chevronUp' : 'chevronDown'} onClick={() => setOpen(!open)}>
            {open ? t('Hide') : t('Read it here')}
          </Button>
        </>
      }
    >
      {open ? page.text === null ? page.error ? <span className="muted">{page.error}</span> : <Spinner /> : <Markdown text={page.text} className="e2e-trace" /> : <span className="muted">{t('Every step with what it did, what the app logged, and the screen at failure, as the framework wrote it.')}</span>}
    </Section>
  );
}

// ---------- one attempt: the recording, the steps, and the two kept in step ----------

function interactive(target: EventTarget | null, selector: string): boolean {
  return target instanceof Element && target.closest(selector) !== null;
}

function AttemptPanel({ attempt, active }: { attempt: E2eSessionAttempt; active: boolean }) {
  const recordings = useMemo(() => recordingsOf(attempt), [attempt]);
  const shots = useMemo(() => shotsOf(attempt), [attempt]);
  const videoRef = useRef<HTMLVideoElement>(null);
  const list = useRef<HTMLOListElement>(null);
  const playable = recordings.findIndex((recording) => recording.url !== null);
  const [recordingIndex, setRecordingIndex] = useState(Math.max(0, playable));
  const [seconds, setSeconds] = useState(0);
  // The step picked by hand: what is highlighted when there is no recording to follow.
  const [picked, setPicked] = useState<number | null>(null);
  const [shotRef, setShotRef] = useState<string | null>(null);
  const [enlarged, setEnlarged] = useState<Shot | null>(null);
  const recording = playable === -1 ? null : (recordings[recordingIndex] ?? null);
  const watching = recording !== null && recording.url !== null;
  const current = watching ? stepAt(attempt.steps, recording, seconds) : (attempt.steps.find((step) => step.index === picked) ?? null);
  const chosenShot = shots.find((shot) => shot.artifact.ref === shotRef) ?? shots.find((shot) => shot.artifact.ref === attempt.failure?.screenshot) ?? shots[0] ?? null;
  const moments: Moment[] = watching ? attempt.steps.flatMap((step) => (offsetIn(step, recording) === null ? [] : [{ step, seconds: offsetIn(step, recording) ?? 0 }])) : [];

  /** The playhead moved. It is only taken up when another step is showing: this is told on every frame. */
  const follow = useCallback(
    (at: number) => {
      if (recording) {
        setSeconds((before) => (stepAt(attempt.steps, recording, before)?.index === stepAt(attempt.steps, recording, at)?.index ? before : at));
      }
    },
    [attempt.steps, recording],
  );

  /** Shows a step: the recording goes to its moment; without one, its screenshot is shown. */
  const select = useCallback(
    (step: E2eSessionStep) => {
      setPicked(step.index);

      const taken = shots.find((shot) => shot.step?.index === step.index);

      if (taken) {
        setShotRef(taken.artifact.ref);
      }

      if (playable === -1) {
        return;
      }

      const wanted = recordingFor(step, recordings, recordingIndex);
      const offset = recordings[wanted] ? offsetIn(step, recordings[wanted]) : null;

      if (offset === null) {
        return;
      }

      setRecordingIndex(wanted);
      setSeconds(offset);
      // Another recording loads first; the seek waits for its metadata.
      requestAnimationFrame(() => {
        const video = videoRef.current;

        if (!video) {
          return;
        }

        const go = () => {
          video.currentTime = offset;
        };

        if (video.readyState >= 1) {
          go();
        } else {
          video.addEventListener('loadedmetadata', go, { once: true });
        }
      });
    },
    [shots, playable, recordings, recordingIndex],
  );

  // The step being shown stays in view in the list, without moving the page.
  useEffect(() => {
    const box = list.current;
    const row = box?.querySelector<HTMLElement>(`[data-step="${current?.index}"]`);

    if (box && row) {
      const top = row.offsetTop - box.offsetTop;

      if (top < box.scrollTop) {
        box.scrollTop = top;
      } else if (top + row.offsetHeight > box.scrollTop + box.clientHeight) {
        box.scrollTop = top + Math.min(row.offsetHeight, box.clientHeight) - box.clientHeight;
      }
    }
  }, [current?.index]);

  // Space plays and pauses; the arrows go to the step before and after.
  useEffect(() => {
    function onKey(event: KeyboardEvent): void {
      if (event.altKey || event.ctrlKey || event.metaKey || interactive(event.target, 'input, textarea, select, [contenteditable], [role="dialog"]')) {
        return;
      }

      const video = videoRef.current;

      if (event.key === ' ' && video && !interactive(event.target, 'button, a')) {
        event.preventDefault();
        void (video.paused ? video.play() : video.pause());
      }

      const move = event.key === 'ArrowDown' || event.key === 'ArrowRight' ? 1 : event.key === 'ArrowUp' || event.key === 'ArrowLeft' ? -1 : 0;

      if (move !== 0 && attempt.steps.length > 0) {
        event.preventDefault();

        const at = current ? attempt.steps.findIndex((step) => step.index === current.index) : -1;
        const next = attempt.steps[Math.min(attempt.steps.length - 1, Math.max(0, at + move))];

        if (next) {
          select(next);
        }
      }
    }

    window.addEventListener('keydown', onKey);

    return () => window.removeEventListener('keydown', onKey);
  }, [attempt.steps, current, select]);

  return (
    <div className="e2e-attempt">
      {attempt.error && <ErrorBlock error={attempt.error} />}
      {attempt.alsoFailed.map((error, index) => (
        <ErrorBlock key={index} error={error} also />
      ))}
      <div className="e2e-session-main">
        <div className="e2e-stage">
          {watching ? (
            <>
              <Player recordings={recordings} index={recordingIndex} onIndex={setRecordingIndex} videoRef={videoRef} steps={attempt.steps} current={current} onTime={follow} onStep={select} />
              <Filmstrip url={recording.url ?? ''} moments={moments} current={current} onStep={select} />
            </>
          ) : (
            <Stills attempt={attempt} shots={shots} chosen={chosenShot} onChoose={(shot) => (shot.step ? select(shot.step) : setShotRef(shot.artifact.ref))} active={active} />
          )}
        </div>
        <section className="panel e2e-steps" aria-label={t('Steps')}>
          <header className="panel-head">
            <h2>{t('Steps')}</h2>
            <span className="tab-count">{attempt.steps.length + attempt.moreSteps}</span>
            <span className="filter-spacer" />
            <span className="muted e2e-keys" title={t('Space plays and pauses; the arrow keys go to the step before and after.')}>
              {watching ? t('space · ↑ ↓') : '↑ ↓'}
            </span>
          </header>
          {attempt.steps.length === 0 ? (
            <p className="muted panel-empty">{t('No steps were recorded for this attempt.')}</p>
          ) : (
            <ol className="e2e-step-list" ref={list}>
              {attempt.steps.map((step) => (
                <StepRow
                  key={step.index}
                  step={step}
                  attempt={attempt}
                  shots={shots}
                  offset={watching ? offsetIn(step, recording) : null}
                  current={step.index === current?.index}
                  onSelect={() => select(step)}
                  onShot={setEnlarged}
                />
              ))}
              {attempt.moreSteps > 0 && <li className="muted e2e-step-cut">{t('{count} more steps were not kept.', { count: attempt.moreSteps })}</li>}
            </ol>
          )}
        </section>
      </div>
      {watching && shots.length > 0 && (
        <Section title={t('Screenshots')} count={shots.length}>
          <div className="e2e-strip e2e-strip-stills">
            {shots.map((shot) => (
              <ShotTile key={shot.artifact.ref} shot={shot} active={active} onSelect={() => setEnlarged(shot)} />
            ))}
          </div>
        </Section>
      )}
      <AtFailure attempt={attempt} active={active} />
      <AppLog lines={attempt.appLog} more={attempt.moreAppLog} onStep={(index) => attempt.steps.filter((step) => step.index === index).forEach(select)} />
      <Files attempt={attempt} active={active} />
      {enlarged && <Enlarged shot={enlarged} onClose={() => setEnlarged(null)} />}
    </div>
  );
}

// ---------- the page ----------

/** One test result as a session to watch: its attempts, the recording beside the steps, the error and what was left behind. */
export function SessionView({ id }: { id: number }) {
  const project = useProject();
  const params = useSearchParams();
  const [acting, setActing] = useState<Acting | null>(null);
  const runId = useRef<number | null>(null);
  const session = useRemote(
    () => api.get<E2eSession>(`${BASE}/results/${id}`),
    [id],
    // A file that arrives, a job that ends, a link to an issue: the session shows each.
    (event) => event.projectId === project.id && ((event.kind === 'e2e.artifact' && event.payload.runId === runId.current) || event.kind === 'e2e.job' || event.kind === 'e2e.updated' || event.kind === 'e2e.run'),
  );
  const data: E2eSession | undefined = session.data;
  runId.current = data?.run.id ?? null;

  if (!data) {
    return (
      <div className="view e2e-view">
        <Link href={`/p/${project.key}/tests`} className="e2e-back">
          <BackIcon size={14} />
          {t('Tests')}
        </Link>
        {session.error ? <Empty icon="alert" title={session.error} /> : <Spinner />}
      </div>
    );
  }

  const { run, result, attempts } = data;
  const wanted = Number(params.get('attempt') ?? '') - 1;
  const attempt = attempts[wanted] ?? attempts.at(-1);
  const chosen = attempt ? attempts.indexOf(attempt) : -1;
  const failed = result.status === 'failed' || result.status === 'flaky';

  return (
    <div className="view e2e-view e2e-session">
      <header className="view-head">
        <div className="e2e-head-title">
          <Link href={runPath(project.key, run.id)} className="e2e-back">
            <BackIcon size={14} />
            {run.name ?? t('Run {id}', { id: run.id })}
          </Link>
          <h1>{result.title}</h1>
          <span className="muted e2e-sub">
            <StatusMark status={result.status} />
            <span className="mono">
              {result.file}
              {result.line ? `:${result.line}` : ''}
            </span>
            {result.variant && <Tag>{result.variant}</Tag>}
            {data.tags.map((tag) => (
              <span key={tag} className="e2e-tag">
                {tag}
              </span>
            ))}
            <Origin branch={run.branch} commit={run.commit} />
            <span title={formatDateTime(run.startedAt)}>{duration(result.durationMs)}</span>
          </span>
          {result.issues.length > 0 && (
            <span className="e2e-sub">
              <span className="muted">{t('Verifies')}</span>
              <IssueChips issues={result.issues} failing={result.status === 'failed'} />
            </span>
          )}
        </div>
        <div className="view-head-side">
          {data.jobId !== null && (
            <Link href={jobPath(project.key, data.jobId)} className="e2e-more">
              {t('Job #{id} and its output', { id: data.jobId })}
            </Link>
          )}
          {data.canEdit && failed && (
            <Button icon="plus" onClick={() => setActing({ kind: 'bug', failure: failureOfResult(result, run) })}>
              {data.bugTypeId ? t('Create bug') : t('Create issue')}
            </Button>
          )}
        </div>
      </header>

      {attempts.length > 1 && (
        <Tabs
          tabs={attempts.map((candidate, index) => ({ id: String(index + 1), label: ATTEMPT_TEXT[candidate.status] ? t(ATTEMPT_TEXT[candidate.status], { number: index + 1 }) : `${t('Attempt {number}', { number: index + 1 })} · ${candidate.status}` }))}
          active={String(chosen + 1)}
          onChange={(tab) => setParam('attempt', tab, { replace: true })}
        />
      )}

      {attempt ? (
        <AttemptPanel key={`${id}:${chosen}`} attempt={attempt} active={data.jobActive} />
      ) : (
        <Empty icon="info" title={run.format === 'junit' ? t('A JUnit report tells results, not steps') : t('This test never ran an attempt')}>
          {run.format === 'junit' ? t('There is no session to watch for this run. Post the framework’s JSON report, or run the tests as a job, to get the steps, the recording and the screenshots.') : result.message}
        </Empty>
      )}
      {!attempt && run.format === 'junit' && result.message && <pre className={`e2e-message e2e-is-${result.status}`}>{result.message}</pre>}

      {attempt && <Trace artifactId={data.traceArtifactId} />}
      <Actions project={project} acting={acting} bugTypeId={data.bugTypeId} onClose={() => setActing(null)} onDone={() => void session.reload()} />
    </div>
  );
}
