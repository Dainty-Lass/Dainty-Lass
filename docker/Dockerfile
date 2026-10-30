# syntax=docker/dockerfile:1
#
# DaintyLass server images (docs/hosting-docker.md). Built from the repository root:
#   docker compose build            (compose.yaml), or
#   docker build -f docker/Dockerfile --target server .
#
# Two images, both laid out as the server release is (RD8, .forgejo/workflows/server-release.yml), under
# /opt/reforged, so every path reforged-host writes into the env files is the same in every container:
#   front   the internet-facing Rust services on Debian slim: gateway, allowlist (with nftables), content,
#           admin API, each a container running `reforged-host <service>` from the one server binary (RD26)
#   server  reforged-host (with the Rust 1.14.7 backend and Director compiled in, RU9/RB14) and Wine for the
#           game servers on Debian slim; also the image the host kit's commands run in
#   netem   iproute2 alone, for the `network` profile's injected latency (DN12)
#
# REFORGED_EDITION=legacy builds them for a Dauntless Revived 1.4.4 server (compose.legacy.yaml): the server image
# also holds DR's metagame and deploy server, built in the legacy-revived submodule's own workspace (DS6).
#
# `server-rollback` keeps the Node backend and Director and MongoDB's dump tools for the `rollback` Compose profile
# for one release until RB15 deletes it (docs/hosting-docker.md).

ARG RUST_IMAGE=rust:1-slim-trixie
ARG NODE_IMAGE=node:22-trixie-slim
ARG DEBIAN_IMAGE=debian:trixie-slim
ARG MONGO_IMAGE=mongo:8.0

# ------------------------------------------------------------------ the game modules' Rust sources (DF5)
# Cargo reads the module crates' manifests, and their workspaces', even with every module feature off, and the
# workspace's [patch] entries point the modules' git dependencies at these paths: nothing is fetched over ssh. Only
# what cargo reads; the Node services, native code and game data are other stages' business.
FROM scratch AS module-sources
COPY modules/rustic-isles/Cargo.toml modules/rustic-isles/Cargo.lock /modules/rustic-isles/
COPY modules/rustic-isles/crates/ /modules/rustic-isles/crates/
COPY modules/rustic-isles/modules/ /modules/rustic-isles/modules/
COPY modules/rustic-isles/docs/ /modules/rustic-isles/docs/
COPY modules/rustic-isles/reference/1.14.7/ParadoxBackend/game-data/ /modules/rustic-isles/reference/1.14.7/ParadoxBackend/game-data/
COPY modules/rustic-isles/reference/1.14.7/ParadoxBackend/store-captures.json /modules/rustic-isles/reference/1.14.7/ParadoxBackend/
COPY modules/rustic-isles/reference/1.14.7/ParadoxDirector/game-data/ /modules/rustic-isles/reference/1.14.7/ParadoxDirector/game-data/
COPY modules/rustic-rifts/Cargo.toml modules/rustic-rifts/Cargo.lock /modules/rustic-rifts/
COPY modules/rustic-rifts/crates/ /modules/rustic-rifts/crates/
COPY modules/rustic-rifts/docs/ /modules/rustic-rifts/docs/
COPY modules/rustic-rifts/src/ /modules/rustic-rifts/src/

# ------------------------------------------------------------------ the Rust binaries
FROM ${RUST_IMAGE} AS rust-build
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential pkg-config ca-certificates \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src/rust
# The pinned toolchain (rust-toolchain.toml), in its own layer
COPY rust/rust-toolchain.toml ./
RUN rustup show active-toolchain || rustup toolchain install
COPY rust/ ./
COPY --from=module-sources /modules/ /src/modules/
COPY modules/legacy-revived/ /src/modules/legacy-revived/
# The release workflow's build: the server in one binary, reforged-host, with the gateway, allowlist, content server
# and admin API inside it, and the game modules (all-modules, DF5). The launcher is a workspace member but is not
# built (only the lock file is read).
# REFORGED_EDITION=legacy: the same binary with its `legacy` feature, and legacy-revived's metagame and deploy server.
# REFORGED_MODULES: the server-console features that link the game modules; empty builds none, which still serves.
ARG REFORGED_EDITION=reborn
ARG REFORGED_MODULES=all-modules
RUN --mount=type=cache,id=reforged-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=reforged-cargo-target,target=/src/rust/target \
    --mount=type=cache,id=reforged-revived-target,target=/src/modules/legacy-revived/rust/target \
    features="$REFORGED_MODULES" \
 && case "$REFORGED_EDITION" in \
      reborn) ;; \
      legacy) features="${features:+$features,}legacy" ;; \
      *) echo "REFORGED_EDITION must be reborn or legacy, not $REFORGED_EDITION" >&2; exit 1 ;; \
    esac \
 && set -- \
 && for f in $(echo "$features" | tr , ' '); do set -- "$@" --features "server-console/$f"; done \
 && cargo build --release --locked -p server-console "$@" --bins \
 && mkdir -p /out/bin \
 && cp target/release/reforged-host /out/bin/ \
 && if [ "$REFORGED_EDITION" = legacy ]; then \
      revived=/src/modules/legacy-revived/rust \
      && (cd "$revived" && cargo build --release --locked -p metagame -p deploy --bins) \
      && cp "$revived/target/release/metagame" "$revived/target/release/deploy" /out/bin/; \
    fi

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
COPY --from=rust-build /out/bin/reforged-host /opt/reforged/bin/
COPY --chmod=0755 docker/healthcheck.sh /opt/reforged/bin/reforged-healthcheck
ENV PATH=/opt/reforged/bin:$PATH
WORKDIR /opt/reforged/.workspace
USER reforged
LABEL org.opencontainers.image.title="DaintyLass front door" \
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
COPY --chown=reforged:reforged LICENSE.txt NOTICE.md ADDITIONAL_TERMS.md README.md ./
COPY --chown=reforged:reforged native/ native/
# The host kit finds the server folder by rust/Cargo.toml beside modules/rustic-isles (paths.rs find_repo_root)
COPY --chown=reforged:reforged rust/Cargo.toml rust/
# From the rustic-isles submodule (DS6): its licence files, the runtime and CatalogExporter sources setup checks
# for, and the example game data and store captures pin
COPY --chown=reforged:reforged modules/rustic-isles/ADDITIONAL_TERMS.md modules/rustic-isles/LICENSE modules/rustic-isles/NOTICE.md modules/rustic-isles/
COPY --chown=reforged:reforged modules/rustic-isles/native/ modules/rustic-isles/native/
ARG ISLES=modules/rustic-isles/reference/1.14.7
COPY --chown=reforged:reforged ${ISLES}/ParadoxBackend/game-data/ ${ISLES}/ParadoxBackend/game-data/
COPY --chown=reforged:reforged ${ISLES}/ParadoxBackend/store-captures.json ${ISLES}/ParadoxBackend/
COPY --chown=reforged:reforged ${ISLES}/ParadoxDirector/game-data/ ${ISLES}/ParadoxDirector/game-data/
COPY --from=rust-build /out/bin/ bin/
COPY --chmod=0755 docker/director-entrypoint.sh bin/reforged-director
COPY --chmod=0755 docker/legacy-entrypoint.sh bin/reforged-legacy
COPY --chmod=0755 docker/host-entrypoint.sh bin/reforged-host-docker
COPY --chmod=0755 docker/healthcheck.sh bin/reforged-healthcheck
# The network profile's one-shot (compose.yaml network-init, DN12)
COPY --chmod=0755 docker/network/init.sh bin/reforged-network-init
RUN mkdir -p .workspace "$ISLES/Items_Analysis" "$ISLES/DauntlessEndpointDocumentation" \
 && chown reforged:reforged .workspace "$ISLES/Items_Analysis" "$ISLES/DauntlessEndpointDocumentation"
# The commit the backend names to players (REFORGED_SOURCE_COMMIT, the AGPL source offer). setup reads it from
# git or release.json, neither of which is in the image; the process environment wins over the env file.
ARG REFORGED_COMMIT=""
ENV REFORGED_SOURCE_COMMIT=${REFORGED_COMMIT} \
    METAGAME_GIT_COMMIT=${REFORGED_COMMIT} \
    PATH=/opt/reforged/bin:$PATH
USER reforged
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["reforged-host", "--help"]
LABEL org.opencontainers.image.title="DaintyLass server" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.source="https://git.silentspud.com/silentspud/dainty-lass"

# ------------------------------------------------------------------ netem: latency for the network profile (DN12)
# tc and nothing else, joined to a node's network namespace with CAP_NET_ADMIN (compose.yaml, netem-*)
FROM ${DEBIAN_IMAGE} AS netem
RUN apt-get update && apt-get install -y --no-install-recommends iproute2 && rm -rf /var/lib/apt/lists/*
COPY --chmod=0755 docker/network/netem.sh /usr/local/bin/reforged-netem
ENTRYPOINT ["/usr/local/bin/reforged-netem"]
LABEL org.opencontainers.image.title="DaintyLass network harness latency" \
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
COPY --chown=reforged:reforged rust/ rust/
COPY --chown=reforged:reforged --from=module-sources /modules/ /opt/reforged/modules/
COPY --chmod=0755 docker/test-in-image.sh bin/reforged-test
RUN chown reforged:reforged rust && chmod -R a+rwX /usr/local/cargo /usr/local/rustup
USER reforged
# The server crates' tests, built now so a run starts at once (the downloaded crates stay in the image: a run
# rebuilds nothing and needs no network). The launcher (Tauri, WebKitGTK) is not a server crate and is not
# built; the fuzz targets are a workspace of their own.
RUN cd rust && cargo test --locked --no-run \
      -p protocol -p proxy -p remote -p service-kit -p store -p gateway -p content -p admin -p hostkit -p host \
      -p controller -p backend -p director
# gateway's route-policy tests check /reforged/api against the reference route file
COPY --chown=reforged:reforged modules/rustic-isles/reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts modules/rustic-isles/reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts
ENTRYPOINT ["/usr/bin/tini", "--", "reforged-test"]
CMD []

# ------------------------------------------------------------------ rollback: Node services and MongoDB tools (until RB15)
FROM ${NODE_IMAGE} AS node-build
WORKDIR /src
ARG ISLES=modules/rustic-isles/reference/1.14.7
COPY ${ISLES}/ParadoxBackend/package.json ${ISLES}/ParadoxBackend/package-lock.json ${ISLES}/ParadoxBackend/
COPY ${ISLES}/ParadoxDirector/package.json ${ISLES}/ParadoxDirector/package-lock.json ${ISLES}/ParadoxDirector/
RUN --mount=type=cache,id=reforged-npm,target=/root/.npm \
    (cd "$ISLES/ParadoxBackend" && npm ci --no-audit --no-fund) \
 && (cd "$ISLES/ParadoxDirector" && npm ci --no-audit --no-fund)
COPY ${ISLES}/ ${ISLES}/
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
COPY --chown=node:node LICENSE.txt NOTICE.md ADDITIONAL_TERMS.md README.md ./
COPY --chown=node:node native/ native/
COPY --chown=node:node rust/Cargo.toml rust/
COPY --chown=node:node modules/rustic-isles/ADDITIONAL_TERMS.md modules/rustic-isles/LICENSE modules/rustic-isles/NOTICE.md modules/rustic-isles/
COPY --chown=node:node modules/rustic-isles/native/ modules/rustic-isles/native/
COPY --chown=node:node --from=node-build /src/modules/rustic-isles/reference/1.14.7/ modules/rustic-isles/reference/1.14.7/
COPY --from=rust-build /out/bin/ bin/
COPY --chmod=0755 docker/director-entrypoint.sh bin/reforged-director
COPY --chmod=0755 docker/legacy-entrypoint.sh bin/reforged-legacy
COPY --chmod=0755 docker/host-entrypoint.sh bin/reforged-host-docker
COPY --chmod=0755 docker/healthcheck.sh bin/reforged-healthcheck
COPY --chmod=0755 docker/network/init.sh bin/reforged-network-init
ARG ISLES=modules/rustic-isles/reference/1.14.7
RUN mkdir -p .workspace "$ISLES/Items_Analysis" "$ISLES/DauntlessEndpointDocumentation" \
 && chown node:node .workspace "$ISLES/Items_Analysis" "$ISLES/DauntlessEndpointDocumentation"
ARG REFORGED_COMMIT=""
ENV REFORGED_SOURCE_COMMIT=${REFORGED_COMMIT} \
    METAGAME_GIT_COMMIT=${REFORGED_COMMIT} \
    MONGODB_URI=mongodb://127.0.0.1:27017/?replicaSet=rs0 \
    MONGODB_DB=reforged \
    MONGODB_APP_NAME=reforged-backend \
    MONGODB_TRANSACTION_MAX_COMMIT_TIME_MS=15000 \
    PATH=/opt/reforged/bin:$PATH
USER node
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["reforged-host", "--help"]
LABEL org.opencontainers.image.title="DaintyLass server (Node rollback)" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.source="https://git.silentspud.com/silentspud/dainty-lass"
