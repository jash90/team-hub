// When things happen, said the way the schedule's page says it: the next run of a slot in
// the schedule's time zone, and the moments of a result and of a failed attempt.

import { useEffect, useState } from 'react';

import type { ScheduleSettings } from '../../bindings/ScheduleSettings.ts';
import { currentLanguage, formatDate, formatShortDate, t } from '../../i18n/index.ts';

export type DailySlot = 'morning' | 'afternoon';

// The Polish for a day's name is how it answers "when?" ("w poniedziałek").
const DAYS = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'];
const SHORT_DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const DAY_MS = 24 * 3600 * 1000;

/** The zone this browser is in, as an IANA name. */
export function browserZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone;
}

/** The day of the week (Monday is 0) and the minute of the day in a time zone; null for a zone this browser does not know. */
function wallClock(now: Date, timeZone: string): { weekday: number; minutes: number } | null {
  try {
    const parts = new Intl.DateTimeFormat('en-GB', { timeZone, weekday: 'short', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' }).formatToParts(now);
    const part = (type: string) => parts.find((candidate) => candidate.type === type)?.value ?? '';
    const weekday = SHORT_DAYS.indexOf(part('weekday'));

    return weekday === -1 ? null : { weekday, minutes: Number(part('hour')) * 60 + Number(part('minute')) };
  } catch {
    return null;
  }
}

/**
 * When a slot runs next, e.g. "next: tomorrow 10:00". The server decides when a slot really
 * runs (it also knows what has run already); this says what its rule comes to for a slot
 * whose time today has not passed, or has passed and been dealt with.
 */
export function nextRun(settings: ScheduleSettings, slot: DailySlot, now: Date): string | null {
  const clock = wallClock(now, settings.timezone);
  const [hour, minute] = settings[slot].split(':').map(Number);

  if (!clock || Number.isNaN(hour) || Number.isNaN(minute)) {
    return null;
  }

  for (let ahead = 0; ahead <= 7; ahead += 1) {
    const weekday = (clock.weekday + ahead) % 7;
    const working = !settings.weekdaysOnly || weekday < 5;

    if (working && (ahead > 0 || clock.minutes < hour * 60 + minute)) {
      const when = ahead === 0 ? t('today') : ahead === 1 ? t('tomorrow') : t(DAYS[weekday]);

      return t('next: {when} {time}', { when, time: settings[slot] });
    }
  }

  return null;
}

function time(date: Date): string {
  return date.toLocaleTimeString(currentLanguage() === 'pl' ? 'pl-PL' : 'en-GB', { hour: '2-digit', minute: '2-digit' });
}

/** How many midnights lie between the moment and now, in this browser's zone. */
function daysAgo(date: Date, now: Date): number {
  const midnight = (moment: Date) => new Date(moment.getFullYear(), moment.getMonth(), moment.getDate()).getTime();

  return Math.round((midnight(now) - midnight(date)) / DAY_MS);
}

function weekday(date: Date): string {
  return DAYS[(date.getDay() + 6) % 7];
}

/** "from Tuesday 10:02", for the result that is still on show. */
export function fromMoment(iso: string, now: Date): string {
  const date = new Date(iso);
  const ago = daysAgo(date, now);
  const day = ago === 0 ? t('from today') : ago === 1 ? t('from yesterday') : ago < 7 ? t(`from ${weekday(date)}`) : t('from {date}', { date: formatDate(iso) });

  return `${day} ${time(date)}`;
}

/** "at 16:00" or "on Monday at 16:00", for the attempt that failed. */
export function atMoment(iso: string, now: Date): string {
  const date = new Date(iso);
  const ago = daysAgo(date, now);

  if (ago === 0) {
    return t('at {time}', { time: time(date) });
  }

  if (ago === 1) {
    return t('yesterday at {time}', { time: time(date) });
  }

  return t('on {day} at {time}', { day: ago < 7 ? t(weekday(date)) : formatShortDate(iso), time: time(date) });
}

/** The current time, refreshed often enough for "3 minutes ago" and "next: today 16:00" to stay true. */
export function useNow(): Date {
  const [now, setNow] = useState(() => new Date());

  useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), 30_000);

    return () => window.clearInterval(timer);
  }, []);

  return now;
}
