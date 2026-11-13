// The desktop app shows this interface in its window, beside its own panel of agent threads.
// The app tells a page of its server one thing, by a value it sets before the page's own
// code runs (apps/desktop/src-tauri/src/stage.rs): which runner the app is. Nothing else
// passes between the two: a page cannot call the app.

type Desktop = { runnerId?: unknown };

/** The id of the runner this page is shown by, when it is shown inside the desktop app. */
export function hereRunner(): string | null {
  const desktop = (window as { __TEAM_HUB_DESKTOP__?: Desktop }).__TEAM_HUB_DESKTOP__;

  return typeof desktop?.runnerId === 'string' && desktop.runnerId ? desktop.runnerId : null;
}
