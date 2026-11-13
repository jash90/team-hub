# team-hub as one image: the server binary and the built interface.
#
#   docker build -t team-hub .
#   docker run -p 8080:8080 -v team-hub-data:/data team-hub
#
# Everything worth keeping (database, attachments, the instance key) is in /data: mount a
# volume there. Put TLS in front before exposing it, and set TEAM_HUB_BASE_URL to the address
# people use. On a public address also set TEAM_HUB_SETUP_TOKEN, so that the first account
# goes to you and not to whoever finds the address first.

# ---- the interface ----
FROM node:22-bookworm-slim AS web
WORKDIR /src
COPY web/package.json web/package-lock.json web/
RUN cd web && npm ci
# The build type-checks the modules' screens and checks the dictionary of server messages
# against the Rust sources, so it needs the whole tree.
COPY . .
RUN cd web && npm run build

# ---- the server ----
FROM rust:1-bookworm AS server
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p hub-server

# ---- what runs ----
FROM debian:bookworm-slim
# Certificates for the calls the server makes itself: GitHub, Jira, Confluence, the model provider.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=server /src/target/release/team-hub /app/team-hub
COPY --from=web /src/web/dist /app/web
# Listens on $PORT when the platform sets one, else on 8080.
ENV TEAM_HUB_BIND=0.0.0.0 \
    TEAM_HUB_DATA_DIR=/data \
    TEAM_HUB_WEB_DIR=/app/web
EXPOSE 8080
CMD ["/app/team-hub"]
