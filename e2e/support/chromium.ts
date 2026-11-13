/**
 * Runs the suite on a Chromium that is already on the machine instead of the one Playwright
 * downloads: the web engine attaches over the DevTools protocol (`web({ connect })`) to a
 * browser this module starts from the given executable.
 */

import { spawn, type ChildProcess } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import type { WebConnectOptions } from '@e2e-dev/web';

const STARTUP_MS = 30_000;
const started: { browser: ChildProcess; profile: string }[] = [];

/** Ends the browser with the helper processes it started: they share its process group. */
function stop(browser: ChildProcess): void {
  try {
    if (browser.pid !== undefined) {
      process.kill(-browser.pid, 'SIGKILL');
    }
  } catch {
    browser.kill('SIGKILL');
  }
}

function stopAll(): void {
  for (const { browser, profile } of started.splice(0)) {
    stop(browser);

    try {
      // The browser may still be writing as it dies; a retry or two gets the directory.
      rmSync(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 50 });
    } catch {
      // A leftover profile in the temporary directory harms nothing.
    }
  }
}

/** Starts one headless browser and resolves with its DevTools address. */
function start(executable: string, signal: AbortSignal): Promise<string> {
  const profile = mkdtempSync(path.join(tmpdir(), 'team-hub-e2e-chromium-'));
  const browser = spawn(
    executable,
    [
      '--headless=new',
      '--remote-debugging-port=0',
      `--user-data-dir=${profile}`,
      '--no-first-run',
      '--no-default-browser-check',
      '--disable-gpu',
      '--disable-dev-shm-usage',
      // A test browser talks to the app under test and to nothing else.
      '--disable-background-networking',
      '--disable-component-update',
      '--disable-default-apps',
      '--disable-sync',
      '--disable-client-side-phishing-detection',
      '--disable-features=OptimizationHints,MediaRouter,Translate',
      '--metrics-recording-only',
      '--no-pings',
      // Chromium refuses to start its sandbox as root, which is what CI containers run as.
      ...(process.getuid?.() === 0 ? ['--no-sandbox'] : []),
      'about:blank',
    ],
    // Its own process group: an interrupt of the run does not reach it, and `stop` ends all of it.
    { stdio: ['ignore', 'ignore', 'pipe'], detached: true },
  );

  if (started.length === 0) {
    process.once('exit', stopAll);
  }

  started.push({ browser, profile });
  // However the browser ends, its profile goes with it.
  browser.once('close', () => rmSync(profile, { recursive: true, force: true }));

  return new Promise((resolve, reject) => {
    let output = '';
    const fail = (reason: string): void => {
      stop(browser);
      reject(new Error(`${executable} did not start: ${reason}`));
    };
    const timer = setTimeout(() => fail(`no DevTools address after ${STARTUP_MS} ms`), STARTUP_MS);

    signal.addEventListener('abort', () => fail('cancelled'), { once: true });
    browser.once('error', (error) => fail(error.message));
    browser.once('exit', (code) => fail(`exited with code ${code}: ${output.trim().split('\n').pop() ?? ''}`));
    browser.stderr?.on('data', (chunk: Buffer) => {
      output += chunk.toString();
      const found = /DevTools listening on (ws:\/\/\S+)/.exec(output);

      if (found?.[1] !== undefined) {
        clearTimeout(timer);
        browser.removeAllListeners('exit');
        // The engine keeps the connection; nothing here needs the output any more.
        browser.stderr?.removeAllListeners('data');
        browser.stderr?.resume();
        resolve(found[1]);
      }
    });
  });
}

/** `connect` options for `web()` that attach to a Chromium started from `executable`. */
export function localChromium(executable: string): WebConnectOptions {
  return { cdpEndpoint: (signal) => start(executable, signal) };
}
