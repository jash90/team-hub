import { pl, plPlurals } from './pl.ts';

export type Language = 'en' | 'pl';
type Vars = Record<string, string | number>;

const STORAGE_KEY = 'team-hub:language';

function initialLanguage(): Language {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);

    if (stored === 'en' || stored === 'pl') {
      return stored;
    }
  } catch {
    // Storage can be unavailable; the browser's language still decides.
  }

  return navigator.language.toLowerCase().startsWith('pl') ? 'pl' : 'en';
}

let language: Language = initialLanguage();
const extra: Record<string, string> = {};

document.documentElement.lang = language;

export function currentLanguage(): Language {
  return language;
}

/** Stores the choice and reloads, so every string on screen is in one language. */
export function setLanguage(next: Language): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, next);
  } catch {
    // Without storage the choice lasts until the reload below, which is no choice at all; nothing to do.
  }

  language = next;
  window.location.reload();
}

/** Polish for strings that modules bring with them. */
export function addTranslations(dictionary: Record<string, string>): void {
  Object.assign(extra, dictionary);
}

/** English is written in place; Polish comes from the dictionary. `{name}` is replaced from `vars`. */
export function t(text: string, vars?: Vars): string {
  const template = language === 'pl' ? (pl[text] ?? extra[text] ?? text) : text;

  if (!vars) {
    return template;
  }

  return template.replace(/\{(\w+)\}/g, (whole, name: string) => (name in vars ? String(vars[name]) : whole));
}

function polishForm(count: number, forms: [string, string, string]): string {
  const abs = Math.abs(count);

  if (abs === 1) {
    return forms[0];
  }

  const lastTwo = abs % 100;
  const last = abs % 10;

  return last >= 2 && last <= 4 && (lastTwo < 12 || lastTwo > 14) ? forms[1] : forms[2];
}

/** "1 issue", "5 issues"; in Polish "1 zadanie", "2 zadania", "5 zadań". */
export function countOf(count: number, one: string, many: string): string {
  if (language === 'pl') {
    const forms = plPlurals[one];

    if (forms) {
      return `${count} ${polishForm(count, forms)}`;
    }
  }

  return `${count} ${count === 1 ? one : many}`;
}

function locale(): string {
  return language === 'pl' ? 'pl-PL' : 'en-GB';
}

export function formatDate(iso: string | null | undefined): string {
  if (!iso) {
    return '';
  }

  const date = new Date(iso.length === 10 ? `${iso}T00:00:00` : iso);

  return Number.isNaN(date.getTime()) ? iso : date.toLocaleDateString(locale(), { day: 'numeric', month: 'short', year: 'numeric' });
}

/** "16 Nov": for places with little room, where the year is obvious. */
export function formatShortDate(iso: string | null | undefined): string {
  if (!iso) {
    return '';
  }

  const date = new Date(iso.length === 10 ? `${iso}T00:00:00` : iso);

  return Number.isNaN(date.getTime()) ? iso : date.toLocaleDateString(locale(), { day: 'numeric', month: 'short' });
}

export function formatDateTime(iso: string | null | undefined): string {
  if (!iso) {
    return '';
  }

  const date = new Date(iso);

  return Number.isNaN(date.getTime()) ? iso : date.toLocaleString(locale(), { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
}

const STEPS: [Intl.RelativeTimeFormatUnit, number][] = [
  ['year', 365 * 24 * 3600],
  ['month', 30 * 24 * 3600],
  ['week', 7 * 24 * 3600],
  ['day', 24 * 3600],
  ['hour', 3600],
  ['minute', 60],
];

/** "3 hours ago", "in 2 days". */
export function timeAgo(iso: string | null | undefined): string {
  if (!iso) {
    return '';
  }

  const seconds = (new Date(iso.length === 10 ? `${iso}T00:00:00` : iso).getTime() - Date.now()) / 1000;
  const formatter = new Intl.RelativeTimeFormat(locale(), { numeric: 'auto' });

  for (const [unit, size] of STEPS) {
    if (Math.abs(seconds) >= size) {
      return formatter.format(Math.round(seconds / size), unit);
    }
  }

  return t('just now');
}

export function formatMinutes(minutes: number): string {
  if (minutes <= 0) {
    return '0m';
  }

  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;

  return [hours ? `${hours}h` : '', rest ? `${rest}m` : ''].filter(Boolean).join(' ');
}
