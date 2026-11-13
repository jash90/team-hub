# The landing page as a static site, served by Caddy on $PORT (default 8080).
# Build context: this folder (on Railway: a service with root directory "landing").
#
#   docker build -t team-hub-landing .
#   docker run --rm -p 8080:8080 team-hub-landing

FROM caddy:2-alpine

COPY Caddyfile /etc/caddy/Caddyfile
COPY index.html 404.html robots.txt /srv/
COPY pl/ /srv/pl/
COPY assets/ /srv/assets/

# Fail the build, not the deploy, when the Caddyfile is wrong.
RUN caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile

# Nothing here needs root: the port is above 1024 and nothing is written
# except Caddy's own state, which goes to /tmp.
ENV PORT=8080 \
    XDG_CONFIG_HOME=/tmp/caddy/config \
    XDG_DATA_HOME=/tmp/caddy/data
USER nobody

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s \
  CMD wget -q -O /dev/null "http://127.0.0.1:${PORT}/healthz" || exit 1

CMD ["caddy", "run", "--config", "/etc/caddy/Caddyfile", "--adapter", "caddyfile"]
