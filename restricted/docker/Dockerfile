# syntax=docker/dockerfile:1
#
# Dainty Lass server images (docs/hosting-docker.md). Built with the monorepo root as context:
#   docker compose build            (from restricted/, compose.yaml), or
#   docker build -f restricted/docker/Dockerfile --target server .   (from the monorepo root)
#
# Two images, both laid out as the server release is (RD8, .forgejo/workflows/server-release.yml), under
# /opt/reforged, so every path dainty-host writes into the env files is the same in every container:
#   front   the internet-facing Rust services on Debian slim: gateway, allowlist (with nftables), content,
#           admin API, each a container running `dainty-host <service>` from the one server binary (RD26)
#   server  dainty-host (with the Rust 1.14.7 backend and Director compiled in, RU9/RB14), the libraries
#           host.toml can pin in plugins/ (RD27), and Wine for the game servers on Debian slim; also the image the
#           host kit's commands run in
#   netem   iproute2 alone, for the `network` profile's injected latency (DN12)
#
# `server-rollback` keeps the Node backend and Director and MongoDB's dump tools for the `rollback` Compose profile
# for one release until RB15 deletes it (docs/hosting-docker.md).

ARG RUST_IMAGE=rust:1-slim-trixie
ARG NODE_IMAGE=node:22-trixie-slim
ARG DEBIAN_IMAGE=debian:trixie-slim
ARG MONGO_IMAGE=mongo:8.0

# ------------------------------------------------------------------ the workspace's Rust sources
# What cargo reads: the MIT crates at crates/, the restricted workspace's members (the game modules are in-tree, RD27)
# and the module data their builds and tests read. The Node services, native code, the dashboard and the 1.4.4
# importer are other stages' business.
FROM scratch AS rust-sources
COPY crates/ /src/crates/
COPY restricted/Cargo.toml restricted/Cargo.lock restricted/rust-toolchain.toml restricted/rustfmt.toml restricted/clippy.toml restricted/deny.toml /src/restricted/
COPY restricted/.cargo/ /src/restricted/.cargo/
COPY restricted/crates/ /src/restricted/crates/
COPY restricted/launcher/ /src/restricted/launcher/
COPY restricted/ui/ /src/restricted/ui/
COPY restricted/modules/rustic-isles/crates/ /src/restricted/modules/rustic-isles/crates/
# The 1.14.7 version data the host and the module library compile in (RD27)
COPY restricted/modules/rustic-isles/version-data/ /src/restricted/modules/rustic-isles/version-data/
COPY restricted/modules/rustic-isles/reference/1.14.7/ParadoxBackend/game-data/ /src/restricted/modules/rustic-isles/reference/1.14.7/ParadoxBackend/game-data/
COPY restricted/modules/rustic-isles/reference/1.14.7/ParadoxBackend/store-captures.json /src/restricted/modules/rustic-isles/reference/1.14.7/ParadoxBackend/
COPY restricted/modules/rustic-isles/reference/1.14.7/ParadoxDirector/game-data/ /src/restricted/modules/rustic-isles/reference/1.14.7/ParadoxDirector/game-data/
COPY restricted/modules/rustic-rifts/oracle/ /src/restricted/modules/rustic-rifts/oracle/
COPY restricted/modules/rustic-rifts/docs/ /src/restricted/modules/rustic-rifts/docs/
COPY restricted/modules/rustic-rifts/src/ /src/restricted/modules/rustic-rifts/src/

# ------------------------------------------------------------------ the Rust binaries
FROM ${RUST_IMAGE} AS rust-build
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential pkg-config ca-certificates git \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src/restricted
# The pinned toolchain (rust-toolchain.toml), in its own layer
COPY restricted/rust-toolchain.toml ./
RUN rustup show active-toolchain || rustup toolchain install
COPY --from=rust-sources /src/ /src/
# The release workflow's build: the server in one binary, dainty-host, with the gateway, allowlist, content server
# and admin API inside it, and the game modules (all-modules, DF5). The launcher is a workspace member but is not
# built (only the lock file is read).
# REFORGED_MODULES: the dainty-server features that link the game modules; empty builds none, which still serves.
ARG REFORGED_MODULES=all-modules
RUN --mount=type=cache,id=reforged-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=reforged-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=reforged-cargo-target,target=/src/restricted/target \
    features="$REFORGED_MODULES" \
 && set -- \
 && for f in $(echo "$features" | tr , ' '); do set -- "$@" --features "dainty-server/$f"; done \
 && cargo build --release --locked -p dainty-server "$@" --bins \
 && mkdir -p /out/bin \
 && cp target/release/dainty-host /out/bin/
# The libraries host.toml can pin (RD27), as the release ships them in plugins/; a service's crate joins the list by
# being in the workspace
RUN --mount=type=cache,id=reforged-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=reforged-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=reforged-cargo-target,target=/src/restricted/target \
    set -- \
 && for pkg in rustic-isles-plugin dainty-content-plugin dainty-chat-plugin dainty-gateway-plugin; do \
      if cargo pkgid --locked -p "$pkg" > /dev/null 2>&1; then set -- "$@" -p "$pkg"; fi; \
    done \
 && cargo build --release --locked "$@" --lib \
 && mkdir -p /out/plugins \
 && for lib in rustic_isles_plugin dainty_content_plugin dainty_chat_plugin dainty_gateway_plugin; do \
      if [ -f "target/release/lib$lib.so" ]; then cp "target/release/lib$lib.so" /out/plugins/; fi; \
    done \
 && test -f /out/plugins/librustic_isles_plugin.so

# ------------------------------------------------------------------ front: gateway, allowlist, content, admin API
FROM ${DEBIAN_IMAGE} AS front
# nftables for the allowlist (ALLOWLIST_NFT is /usr/sbin/nft). The helper runs as the reforged user with
# CAP_NET_ADMIN in its bounding set (compose.yaml); nft gets it from its file capability, nothing else does.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates nftables libcap2-bin \
 && setcap cap_net_admin+ep /usr/sbin/nft \
 && apt-get purge -y libcap2-bin && apt-get autoremove -y \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --gid 1000 reforged \
 && useradd --uid 1000 --gid 1000 --home-dir /opt/reforged --no-create-home --shell /usr/sbin/nologin reforged \
 && mkdir -p /opt/reforged/.workspace && chown reforged:reforged /opt/reforged/.workspace
COPY --from=rust-build /out/bin/dainty-host /opt/reforged/bin/
COPY --chmod=0755 restricted/docker/healthcheck.sh /opt/reforged/bin/reforged-healthcheck
ENV PATH=/opt/reforged/bin:$PATH
WORKDIR /opt/reforged/.workspace
USER reforged
LABEL org.opencontainers.image.title="Dainty Lass front door" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.source="https://git.silentspud.com/silentspud/dainty-lass"

# ------------------------------------------------------------------ server: host kit, Rust backend and Director
FROM ${DEBIAN_IMAGE} AS server
# Wine and a virtual display for the Windows game servers (docs/hosting-linux.md); tini reaps them.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates wine wine64 xvfb xauth tini \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --gid 1000 reforged \
 && useradd --uid 1000 --gid 1000 --home-dir /opt/reforged --no-create-home --shell /usr/sbin/nologin reforged
# The reforged user (uid 1000) owns everything a host writes: the workspace and the game-data folders, which
# compose.yaml mounts as volumes (a new volume takes this ownership and these files).
WORKDIR /opt/reforged
COPY --chown=reforged:reforged restricted/LICENSE.txt restricted/NOTICE.md restricted/ADDITIONAL_TERMS.md restricted/README.md ./
COPY --chown=reforged:reforged restricted/native/ native/
# The host kit finds the server folder by Cargo.toml beside modules/rustic-isles (paths.rs find_repo_root)
COPY --chown=reforged:reforged restricted/Cargo.toml ./
# From modules/rustic-isles: its licence files, the runtime and CatalogExporter sources setup checks
# for, and the example game data and store captures pin
COPY --chown=reforged:reforged restricted/modules/rustic-isles/ADDITIONAL_TERMS.md restricted/modules/rustic-isles/LICENSE restricted/modules/rustic-isles/NOTICE.md modules/rustic-isles/
COPY --chown=reforged:reforged restricted/modules/rustic-isles/native/ modules/rustic-isles/native/
ARG ISLES=modules/rustic-isles/reference/1.14.7
COPY --chown=reforged:reforged restricted/${ISLES}/ParadoxBackend/game-data/ ${ISLES}/ParadoxBackend/game-data/
COPY --chown=reforged:reforged restricted/${ISLES}/ParadoxBackend/store-captures.json ${ISLES}/ParadoxBackend/
COPY --chown=reforged:reforged restricted/${ISLES}/ParadoxDirector/game-data/ ${ISLES}/ParadoxDirector/game-data/
COPY --from=rust-build /out/bin/ bin/
COPY --from=rust-build /out/plugins/ plugins/
COPY --chmod=0755 restricted/docker/director-entrypoint.sh bin/reforged-director
COPY --chmod=0755 restricted/docker/host-entrypoint.sh bin/dainty-host-docker
COPY --chmod=0755 restricted/docker/healthcheck.sh bin/reforged-healthcheck
# The network profile's one-shot (compose.yaml network-init, DN12)
COPY --chmod=0755 restricted/docker/network/init.sh bin/reforged-network-init
RUN mkdir -p .workspace "$ISLES/Items_Analysis" "$ISLES/DauntlessEndpointDocumentation" \
 && chown reforged:reforged .workspace "$ISLES/Items_Analysis" "$ISLES/DauntlessEndpointDocumentation"
# The commit the backend names to players (REFORGED_SOURCE_COMMIT, the AGPL source offer). setup reads it from
# git or release.json, neither of which is in the image; the process environment wins over the env file.
ARG REFORGED_COMMIT=""
ENV REFORGED_SOURCE_COMMIT=${REFORGED_COMMIT} \
    PATH=/opt/reforged/bin:$PATH
USER reforged
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["dainty-host", "--help"]
LABEL org.opencontainers.image.title="Dainty Lass server" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.source="https://git.silentspud.com/silentspud/dainty-lass"

# ------------------------------------------------------------------ netem: latency for the network profile (DN12)
# tc and nothing else, joined to a node's network namespace with CAP_NET_ADMIN (compose.yaml, netem-*)
FROM ${DEBIAN_IMAGE} AS netem
RUN apt-get update && apt-get install -y --no-install-recommends iproute2 && rm -rf /var/lib/apt/lists/*
COPY --chmod=0755 restricted/docker/network/netem.sh /usr/local/bin/reforged-netem
ENTRYPOINT ["/usr/local/bin/reforged-netem"]
LABEL org.opencontainers.image.title="Dainty Lass network harness latency" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.source="https://git.silentspud.com/silentspud/dainty-lass"

# ------------------------------------------------------------------ test: every server suite on the server image
# The server image with the Rust toolchain, the workspace's sources and its tests compiled in, so a test run
# (docker/test-in-image.sh, `docker compose run --rm test`, docker/run-tests.sh) checks the code on the same OS,
# Wine and paths production runs on. Never deployed.
FROM server AS test
USER root
# procps: the host kit's start tests send Ctrl+C with kill(1), which the slim base images leave out
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential pkg-config git nftables libcap2-bin procps \
 && setcap cap_net_admin+ep /usr/sbin/nft \
 && rm -rf /var/lib/apt/lists/*
COPY --from=rust-build /usr/local/cargo /usr/local/cargo
COPY --from=rust-build /usr/local/rustup /usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo RUSTUP_HOME=/usr/local/rustup \
    PATH=/usr/local/cargo/bin:/opt/reforged/bin:$PATH \
    CARGO_TERM_COLOR=never
COPY --chown=reforged:reforged --from=rust-sources /src/crates/ /opt/crates/
COPY --chown=reforged:reforged --from=rust-sources /src/restricted/ /opt/reforged/
COPY --chmod=0755 restricted/docker/test-in-image.sh bin/reforged-test
RUN chmod -R a+rwX /usr/local/cargo /usr/local/rustup
USER reforged
# The server crates' tests, built now so a run starts at once (the downloaded crates stay in the image: a run
# rebuilds nothing and needs no network). The launcher (Tauri, WebKitGTK) is not a server crate and is not
# built; the fuzz targets are a workspace of their own.
RUN cargo test --locked --no-run \
      -p dainty-protocol -p dainty-proxy -p dainty-remote -p dainty-service-kit -p dainty-store -p dainty-gateway -p dainty-content -p dainty-admin -p dainty-hostkit -p dainty-host \
      -p dainty-controller
# gateway's route-policy tests check /reforged/api against the reference route file
COPY --chown=reforged:reforged restricted/modules/rustic-isles/reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts modules/rustic-isles/reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts
ENTRYPOINT ["/usr/bin/tini", "--", "reforged-test"]
CMD []

# ------------------------------------------------------------------ rollback: Node services and MongoDB tools (until RB15)
FROM ${NODE_IMAGE} AS node-build
WORKDIR /src
ARG ISLES=modules/rustic-isles/reference/1.14.7
COPY restricted/${ISLES}/ParadoxBackend/package.json restricted/${ISLES}/ParadoxBackend/package-lock.json ${ISLES}/ParadoxBackend/
COPY restricted/${ISLES}/ParadoxDirector/package.json restricted/${ISLES}/ParadoxDirector/package-lock.json ${ISLES}/ParadoxDirector/
RUN --mount=type=cache,id=reforged-npm,target=/root/.npm \
    (cd "$ISLES/ParadoxBackend" && npm ci --no-audit --no-fund) \
 && (cd "$ISLES/ParadoxDirector" && npm ci --no-audit --no-fund)
COPY restricted/${ISLES}/ ${ISLES}/
RUN (cd "$ISLES/ParadoxBackend" && npm run build) \
 && (cd "$ISLES/ParadoxDirector" && npm run build) \
 && find . -type d -name .bin -path '*/node_modules/*' -prune -exec rm -rf {} +

FROM ${MONGO_IMAGE} AS mongo-tools

FROM ${NODE_IMAGE} AS server-rollback
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates wine wine64 xvfb xauth tini \
 && rm -rf /var/lib/apt/lists/*
COPY --from=mongo-tools /usr/bin/mongodump /usr/bin/mongorestore /usr/bin/
WORKDIR /opt/reforged
COPY --chown=node:node restricted/LICENSE.txt restricted/NOTICE.md restricted/ADDITIONAL_TERMS.md restricted/README.md ./
COPY --chown=node:node restricted/native/ native/
COPY --chown=node:node restricted/Cargo.toml ./
COPY --chown=node:node restricted/modules/rustic-isles/ADDITIONAL_TERMS.md restricted/modules/rustic-isles/LICENSE restricted/modules/rustic-isles/NOTICE.md modules/rustic-isles/
COPY --chown=node:node restricted/modules/rustic-isles/native/ modules/rustic-isles/native/
COPY --chown=node:node --from=node-build /src/modules/rustic-isles/reference/1.14.7/ modules/rustic-isles/reference/1.14.7/
COPY --from=rust-build /out/bin/ bin/
COPY --chmod=0755 restricted/docker/director-entrypoint.sh bin/reforged-director
COPY --chmod=0755 restricted/docker/host-entrypoint.sh bin/dainty-host-docker
COPY --chmod=0755 restricted/docker/healthcheck.sh bin/reforged-healthcheck
COPY --chmod=0755 restricted/docker/network/init.sh bin/reforged-network-init
ARG ISLES=modules/rustic-isles/reference/1.14.7
RUN mkdir -p .workspace "$ISLES/Items_Analysis" "$ISLES/DauntlessEndpointDocumentation" \
 && chown node:node .workspace "$ISLES/Items_Analysis" "$ISLES/DauntlessEndpointDocumentation"
ARG REFORGED_COMMIT=""
ENV REFORGED_SOURCE_COMMIT=${REFORGED_COMMIT} \
    MONGODB_URI=mongodb://127.0.0.1:27017/?replicaSet=rs0 \
    MONGODB_DB=reforged \
    MONGODB_APP_NAME=reforged-backend \
    MONGODB_TRANSACTION_MAX_COMMIT_TIME_MS=15000 \
    PATH=/opt/reforged/bin:$PATH
USER node
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["dainty-host", "--help"]
LABEL org.opencontainers.image.title="Dainty Lass server (Node rollback)" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.source="https://git.silentspud.com/silentspud/dainty-lass"
