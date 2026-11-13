# Connecting the desktop app: one sign-in

The desktop app works as a person: it reads their issues, claims the runs sent to their
computer, pushes in their name. For that it needs an API token of theirs. This is how it gets
one without anybody creating a token by hand and pasting it, and without the app ever seeing
a password.

The app's first screen asks for an e-mail address and a password, which is the plain way
(`apps/desktop/README.md`, "Connecting to a server"). This is the other way, under **Other
ways to connect → Sign in at the tracker**: for whoever will not type a password into an app,
and for a server that one day signs people in some other way than by password.

It is the shape of the OAuth device flow (RFC 8628), cut down to what one server and its own
app need.

## What happens

1. **The app asks.** `POST /api/desktop/pairings` with `{ name, runnerId }`, signed in as
   nobody. The server answers `{ id, code, secret, expiresAt }`:
   - `secret`: 32 random bytes in hex. The app keeps it in memory and shows it to no one; the
     server keeps only its SHA-256.
   - `code`: 8 characters from an alphabet without look-alikes (no `I`, `O`, `0`, `1`). The
     app shows it; so will the server.
   - The request lives 10 minutes.
2. **The person says yes.** The app opens `/desktop/pair?code=…` in the tracker it has in its
   window. Signed out, that address shows the sign-in form first and the question after it.
   The page shows the app's name and the code, and has two buttons:
   `POST /api/desktop/pairings/{code}/approve` and `…/decline`. Saying yes records who said
   it. No token exists yet.
3. **The app collects.** Every 2 seconds: `POST /api/desktop/pairings/{id}/token` with
   `{ secret }`.
   - `202 { "status": "pending" }` while nobody has answered.
   - `200 { "status": "approved", "token", "tokenId" }` once: the token is created at this
     moment, named after the app, and the request is deleted in the same transaction.
   - `410` when the request ran out of time, was declined, was collected already, or never
     existed.
   - `403` when the secret is not the one the request was made with.
4. **The app connects** with that token exactly as with a pasted one: the token goes to the
   keychain, the address to the settings. The token's id goes to the keychain too.
5. **Disconnecting** removes the token on the server (`DELETE /api/me/tokens/{id}`) when the
   app was given it this way and the server can be reached. A token that was pasted is left
   alone: its person made it and may use it elsewhere.

## Why it is safe to let anybody ask

- **Asking gets nothing.** A request is a row with a digest in it. It becomes a token only
  after a signed-in person has said yes to that very code, and only for whoever holds the
  secret.
- **Only a person in a browser can say yes.** Showing, approving and declining a request
  need a session, with its CSRF token; an API token is refused (`403`). A token that leaked
  cannot make itself more tokens.
- **The code is not the secret.** What the person sees and types cannot collect the token;
  what collects the token is never on a screen. The secret is compared by digest.
- **Once.** Collecting deletes the request; two collections at the same moment get one token
  between them. Somebody else cannot approve or decline a request that has an approver.
- **Nothing lingers.** A request nobody collects leaves no token behind; a request whose
  approver was deactivated in the meantime is refused. Expired rows are deleted when the next
  request is made.
- **Bounded.** At most 20 requests wait at once; the next gets `429`. The table cannot be
  filled by somebody asking in a loop.

## What it does not protect against

- **A link somebody sends you.** Anyone can start a request and send its
  `/desktop/pair?code=…` address to a person. If that person approves it, the sender's app
  gets their token. This is the known weakness of every device flow. The page says what it
  can: the name is the app's own claim, and the question is to be answered with yes only by
  someone who started it themselves, on their own computer, a moment ago, and sees the same
  code there. A token given by mistake is listed under **Settings → Profile and tokens** by
  the app's name and can be deleted there.
- **Twenty requests held open.** Whoever keeps 20 requests waiting keeps everyone else from
  starting one, ten minutes at a time. Nothing is exposed by that, and the app still
  connects with a pasted token (**Advanced** on its first screen).
- **A shared runner.** `hub-runner` and machines without a window still take their token
  from `TEAM_HUB_TOKEN` or `hub connect`: there is nobody there to say yes to.

## Where the code is

| Part | File |
| --- | --- |
| The table | `crates/hub-app/src/schema.rs` (`desktop_pairing`) |
| The rules | `crates/hub-app/src/services/pairing.rs` |
| The routes | `crates/hub-server/src/api/pairing.rs` |
| Their tests | `crates/hub-server/tests/pairing.rs` |
| The page | `web/src/views/pair.tsx` |
| The app's side | `apps/desktop/core/src/app.rs` (`pair_start`, `pair_wait`, `pair_cancel`, `disconnect`) |
| Its tests | `apps/desktop/core/tests/desktop.rs` |
| The first screen | `apps/desktop/ui/src/views/connect.tsx` |
