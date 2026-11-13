# The cloud runner as one image: the headless core (hub-runner), the agent helper it starts
# for each thread, and what test jobs need to run a browser.
#
#   docker build -f apps/runner/Dockerfile -t team-hub-runner .      # from the repository root
#   docker run -v team-hub-runner-data:/data \
#     -e TEAM_HUB_URL=… -e TEAM_HUB_TOKEN=… -e GITHUB_TOKEN=… -e ANTHROPIC_API_KEY=… team-hub-runner
#
# Clones, working trees, the browsers test suites install and the runner's state are in
# /data: mount a volume there, or every restart clones and downloads again and the runner
# comes back as a new one. It listens on nothing; see apps/runner/README.md for the variables.

# ---- the runner ----
FROM rust:1-bookworm AS runner
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p hub-desktop-core --bin hub-runner

# ---- the agent helper ----
# Its TypeScript is run by Node as it is (22.18 or newer), so there is nothing to build:
# the sources and the production dependencies, among them the Claude Agent SDK.
FROM node:22-bookworm-slim AS sidecar
WORKDIR /sidecar
COPY apps/desktop/sidecar/package.json apps/desktop/sidecar/package-lock.json ./
RUN npm ci --omit=dev
COPY apps/desktop/sidecar/src ./src

# ---- what runs ----
FROM node:22-bookworm-slim
# - git and certificates: clones, fetches and pushes over HTTPS, and the calls to the server.
# - bubblewrap and socat: the sandbox of an agent thread's shell, where the platform lets it
#   make namespaces; the runner tries it at start and gives threads no shell when it cannot.
# - the fonts and libraries Chromium needs: what `npx playwright install-deps chromium`
#   installs on Debian 12. The browser itself is downloaded by each suite into /data/browsers.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates git bubblewrap socat \
        xvfb fonts-noto-color-emoji fonts-unifont libfontconfig1 libfreetype6 xfonts-scalable fonts-liberation \
        fonts-ipafont-gothic fonts-wqy-zenhei fonts-tlwg-loma-otf fonts-freefont-ttf \
        libasound2 libatk-bridge2.0-0 libatk1.0-0 libatspi2.0-0 libcairo2 libcups2 libdbus-1-3 libdrm2 libgbm1 \
        libglib2.0-0 libnspr4 libnss3 libpango-1.0-0 libx11-6 libxcb1 libxcomposite1 libxdamage1 libxext6 libxfixes3 \
        libxkbcommon0 libxrandr2 \
    && rm -rf /var/lib/apt/lists/* \
    # Test jobs run a repository's own code: as this user, never as the runner's (root).
    && useradd --system --create-home --home-dir /home/job --shell /usr/sbin/nologin job
WORKDIR /app
COPY --from=runner /src/target/release/hub-runner /app/hub-runner
COPY --from=sidecar /sidecar /app/sidecar
ENV RUNNER_DATA_DIR=/data \
    RUNNER_JOB_USER=job \
    TEAM_HUB_SIDECAR="node /app/sidecar/src/main.ts"
# The runner is the container's first process on purpose: it removes its tokens from its own
# environment at start, which a wrapper in front of it would still hold for anyone to read.
# It ends within ten seconds of SIGTERM.
CMD ["/app/hub-runner"]
