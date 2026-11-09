# The dashboard as a container. Build context: the repository root, whose bun.lock covers ui/ and launcher/.
#   docker build -f ui/docker/dashboard.Dockerfile -t dainty-dash .
# It reaches the admin API and backend on loopback, so run it in the server's network namespace
# (network_mode: "service:<backend>" in Compose, or --network host) and mount /data for the database.
FROM oven/bun:1.4.4 AS build
WORKDIR /src
COPY package.json bun.lock bunfig.toml ./
# A frozen install needs every workspace's manifest
COPY launcher/package.json launcher/
COPY ui ui
RUN bun install --frozen-lockfile
WORKDIR /src/ui
RUN bun run theme:build && bun run --filter @dainty/web build

FROM oven/bun:1.4.4
WORKDIR /app
ENV NODE_ENV=production DASHBOARD_DATA_DIR=/data DASHBOARD_MIGRATIONS_DIR=/app/drizzle PORT=3000
COPY --from=build /src/ui/apps/web/dist/standalone ./
COPY --from=build /src/ui/apps/web/drizzle ./drizzle
VOLUME /data
EXPOSE 3000
USER bun
CMD ["bun", "server.js"]
