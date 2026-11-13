import { useSyncExternalStore, type AnchorHTMLAttributes, type MouseEvent } from 'react';

const listeners = new Set<() => void>();

function notify(): void {
  for (const listener of listeners) {
    listener();
  }
}

window.addEventListener('popstate', notify);

function subscribe(listener: () => void): () => void {
  listeners.add(listener);

  return () => listeners.delete(listener);
}

export function navigate(to: string, options: { replace?: boolean } = {}): void {
  if (to === window.location.pathname + window.location.search) {
    return;
  }

  if (options.replace) {
    window.history.replaceState(null, '', to);
  } else {
    window.history.pushState(null, '', to);
  }

  notify();
}

export function usePath(): string {
  return useSyncExternalStore(subscribe, () => window.location.pathname);
}

export function useSearchParams(): URLSearchParams {
  const search = useSyncExternalStore(subscribe, () => window.location.search);

  return new URLSearchParams(search);
}

/** Sets or removes one query parameter, keeping the path and the others. */
export function setParam(name: string, value: string | null, options: { replace?: boolean } = {}): void {
  const search = new URLSearchParams(window.location.search);

  if (value === null || value === '') {
    search.delete(name);
  } else {
    search.set(name, value);
  }

  const text = search.toString();
  navigate(window.location.pathname + (text ? `?${text}` : ''), options);
}

export type Params = Record<string, string>;

/** Matches `/p/:key/settings/:tab?` style patterns; `*` at the end takes the rest. */
export function matchRoute(pattern: string, path: string): Params | null {
  const wanted = pattern.split('/').filter(Boolean);
  const given = path.split('/').filter(Boolean);
  const params: Params = {};

  for (let index = 0; index < wanted.length; index += 1) {
    const part = wanted[index];

    if (part === '*') {
      params.rest = given.slice(index).map(decodeURIComponent).join('/');

      return params;
    }

    const optional = part.endsWith('?');
    const value = given[index];

    if (value === undefined) {
      if (optional) {
        continue;
      }

      return null;
    }

    if (part.startsWith(':')) {
      params[part.slice(1).replace('?', '')] = decodeURIComponent(value);
    } else if (part !== value) {
      return null;
    }
  }

  return given.length > wanted.length ? null : params;
}

type LinkProps = AnchorHTMLAttributes<HTMLAnchorElement> & { href: string };

export function Link({ href, onClick, children, ...rest }: LinkProps) {
  function handle(event: MouseEvent<HTMLAnchorElement>): void {
    onClick?.(event);

    const plain = event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey;

    if (event.defaultPrevented || !plain || rest.target === '_blank' || !href.startsWith('/')) {
      return;
    }

    event.preventDefault();
    navigate(href);
  }

  return (
    <a href={href} onClick={handle} {...rest}>
      {children}
    </a>
  );
}
