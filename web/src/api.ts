export class ApiError extends Error {
  status: number;
  code: string;

  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.code = code;
  }
}

let csrfToken: string | null = null;

export function setCsrf(token: string | null): void {
  csrfToken = token;
}

type ErrorBody = { error?: { code?: string; message?: string } };

function parse(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

async function send<T>(method: string, path: string, body?: BodyInit, headers: Record<string, string> = {}): Promise<T> {
  if (csrfToken && method !== 'GET') {
    headers['x-csrf-token'] = csrfToken;
  }

  const response = await fetch(path, { method, headers, body, credentials: 'same-origin' });
  const text = await response.text();
  const data = text ? parse(text) : null;

  if (!response.ok) {
    const error = (data as ErrorBody | null)?.error;

    throw new ApiError(response.status, error?.code ?? 'error', error?.message ?? (typeof data === 'string' && data ? data : response.statusText));
  }

  return data as T;
}

function json<T>(method: string, path: string, body: unknown): Promise<T> {
  return send<T>(method, path, JSON.stringify(body), { 'content-type': 'application/json' });
}

export const api = {
  get: <T>(path: string) => send<T>('GET', path),
  post: <T>(path: string, body: unknown = {}) => json<T>('POST', path, body),
  patch: <T>(path: string, body: unknown) => json<T>('PATCH', path, body),
  put: <T>(path: string, body: unknown) => json<T>('PUT', path, body),
  del: <T>(path: string) => send<T>('DELETE', path),
  /** The file is the request body; its name travels in a header. */
  upload: <T>(path: string, file: File) =>
    send<T>('POST', path, file, { 'content-type': file.type || 'application/octet-stream', 'x-filename': encodeURIComponent(file.name) }),
};

/** `?a=1&b=two`, skipping empty values. */
export function query(params: Record<string, string | number | boolean | null | undefined>): string {
  const search = new URLSearchParams();

  for (const [key, value] of Object.entries(params)) {
    if (value !== null && value !== undefined && value !== '') {
      search.set(key, String(value));
    }
  }

  const text = search.toString();

  return text ? `?${text}` : '';
}

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
