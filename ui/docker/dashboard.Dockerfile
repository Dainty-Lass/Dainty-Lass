# The dashboard as a container. Build context: the repository root.
#   docker build -f docker/dashboard.Dockerfile -t dainty-dash .
# It reaches the admin API and backend on loopback, so run it in the server's network namespace
# (network_mode: "service:<backend>" in Compose, or --network host) and mount /data for the database.
FROM oven/bun:1.4.4 AS build
WORKDIR /src
COPY package.json bun.lock bunfig.toml ./
COPY patches patches
COPY packages packages
COPY apps apps
RUN bun install --frozen-lockfile
RUN bun run theme:build && bun run --filter @dainty/dashboard build

FROM oven/bun:1.4.4
WORKDIR /app
ENV NODE_ENV=production DASHBOARD_DATA_DIR=/data DASHBOARD_MIGRATIONS_DIR=/app/drizzle PORT=3000
COPY --from=build /src/apps/dashboard/dist/standalone ./
COPY --from=build /src/apps/dashboard/drizzle ./drizzle
VOLUME /data
EXPOSE 3000
USER bun
CMD ["bun", "server.js"]
