# syntax=docker/dockerfile:1
#
# Dainty Lass server images (docs/hosting-docker.md). Built with the monorepo root as context:
#   docker compose build            (from restricted/, compose.yaml), or
#   docker build -f restricted/docker/Dockerfile --target server .   (from the monorepo root)
#
# Two images, both laid out as the server release is (RD8, .forgejo/workflows/server-release.yml), under
# /opt/dainty-lass, so every path dainty-host writes into the env files is the same in every container:
#   front   the internet-facing Rust services on Debian slim: gateway, allowlist (with nftables), content,
#           admin API, each a container running `dainty-host <service>` from the one server binary (RD26)
#   server  dainty-host (with the Rust 1.14.7 backend and Director compiled in, RU9/RB14), the libraries
#           settings.toml can pin in plugins/ (RD27), and Wine for the game servers on Debian slim; also the image the
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
COPY Cargo.toml Cargo.lock /src/
COPY .cargo/ /src/.cargo/
COPY crates/ /src/crates/
# The launcher's version, which the launcher kit's build reads (the gateway's tests take the kit)
COPY launcher/package.json /src/launcher/
COPY restricted/Cargo.toml restricted/Cargo.lock restricted/rust-toolchain.toml restricted/rustfmt.toml restricted/clippy.toml restricted/deny.toml /src/restricted/
COPY restricted/.cargo/ /src/restricted/.cargo/
COPY restricted/crates/ /src/restricted/crates/
COPY restricted/launcher/ /src/restricted/launcher/
COPY restricted/modules/rustic-isles/crates/ /src/restricted/modules/rustic-isles/crates/
# The 1.14.7 version data the parity tests read; the server stage ships it in data/ (RD27)
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
 && apt-get install -y --no-install-recommends build-essential pkg-config ca-certificates git gcc-mingw-w64-x86-64 \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src/restricted
# The pinned toolchain (rust-toolchain.toml), in its own layer
COPY restricted/rust-toolchain.toml ./
RUN (rustup show active-toolchain || rustup toolchain install) && rustup target add x86_64-pc-windows-gnu
COPY --from=rust-sources /src/ /src/
# The release workflow's build: the server in one binary, dainty-host, with the gateway, allowlist, content server
# and admin API inside it, and the game modules (all-modules, DF5). The launcher is a workspace member but is not
# built (only the lock file is read).
# DAINTY_MODULES: the dainty-server features that link the game modules; empty builds none, which still serves.
ARG DAINTY_MODULES=all-modules
RUN --mount=type=cache,id=dainty-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=dainty-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=dainty-cargo-target,target=/src/target \
    features="$DAINTY_MODULES" \
 && set -- \
 && for f in $(echo "$features" | tr , ' '); do set -- "$@" --features "dainty-server/$f"; done \
 && cargo build --release --locked --manifest-path ../Cargo.toml -p dainty-server "$@" --bins \
 && mkdir -p /out/bin \
 && cp ../target/release/dainty-host /out/bin/
# The libraries settings.toml can pin (RD27), as the release ships them in plugins/, all from the MIT root: the 1.14.7
# module and the services; and the legacy importer, as the release ships it in bin/
RUN --mount=type=cache,id=dainty-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=dainty-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=dainty-cargo-target,target=/src/restricted/target \
    --mount=type=cache,id=dainty-cargo-target,target=/src/target \
    cargo build --release --locked --manifest-path ../Cargo.toml \
      -p rustic-isles-1-14-7-plugin -p dainty-content-plugin -p dainty-chat-plugin -p dainty-gateway-plugin --lib \
 && cargo build --release --locked --manifest-path ../Cargo.toml -p dainty-legacy --bin dainty-legacy \
 && cp ../target/release/dainty-legacy /out/bin/ \
 && cargo build --release --locked --manifest-path ../Cargo.toml -p dainty-inject --target x86_64-pc-windows-gnu \
 && cp ../target/x86_64-pc-windows-gnu/release/dainty-inject.exe /out/bin/ \
 && mkdir -p /out/plugins \
 && for lib in rustic_isles_1_14_7_plugin dainty_content_plugin dainty_chat_plugin dainty_gateway_plugin; do \
      cp "../target/release/lib$lib.so" /out/plugins/; \
    done

# ------------------------------------------------------------------ front: gateway, allowlist, content, admin API
FROM ${DEBIAN_IMAGE} AS front
# nftables for the allowlist (ALLOWLIST_NFT is /usr/sbin/nft). The helper runs as the dainty user with
# CAP_NET_ADMIN in its bounding set (compose.yaml); nft gets it from its file capability, nothing else does.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates nftables libcap2-bin \
 && setcap cap_net_admin+ep /usr/sbin/nft \
 && apt-get purge -y libcap2-bin && apt-get autoremove -y \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --gid 1000 dainty \
 && useradd --uid 1000 --gid 1000 --home-dir /opt/dainty-lass --no-create-home --shell /usr/sbin/nologin dainty \
 && mkdir -p /opt/dainty-lass/.workspace && chown dainty:dainty /opt/dainty-lass/.workspace
COPY --from=rust-build /out/bin/dainty-host /opt/dainty-lass/bin/
COPY --chmod=0755 restricted/docker/healthcheck.sh /opt/dainty-lass/bin/dainty-healthcheck
ENV PATH=/opt/dainty-lass/bin:$PATH
WORKDIR /opt/dainty-lass/.workspace
USER dainty
LABEL org.opencontainers.image.title="Dainty Lass front door" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.source="https://git.silentspud.com/silentspud/dainty-lass"

# ------------------------------------------------------------------ server: host kit, Rust backend and Director
FROM ${DEBIAN_IMAGE} AS server
# Wine and a virtual display for the Windows game servers (docs/hosting-linux.md); tini reaps them.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates wine wine64 xvfb xauth tini \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --gid 1000 dainty \
 && useradd --uid 1000 --gid 1000 --home-dir /opt/dainty-lass --no-create-home --shell /usr/sbin/nologin dainty
# The dainty user (uid 1000) owns everything a host writes: the workspace and the game-data folders, which
# compose.yaml mounts as volumes (a new volume takes this ownership and these files).
WORKDIR /opt/dainty-lass
COPY --chown=dainty:dainty restricted/LICENSE.txt restricted/NOTICE.md restricted/ADDITIONAL_TERMS.md restricted/README.md ./
COPY --chown=dainty:dainty native/ native/
# The host kit finds the server folder by Cargo.toml beside modules/rustic-isles (paths.rs find_repo_root)
COPY --chown=dainty:dainty restricted/Cargo.toml ./
# From modules/rustic-isles: its licence files, and the example game data and store captures pin
COPY --chown=dainty:dainty restricted/modules/rustic-isles/ADDITIONAL_TERMS.md restricted/modules/rustic-isles/LICENSE restricted/modules/rustic-isles/NOTICE.md modules/rustic-isles/
# The 1.14.7 version facts and Mystic Paradox's design, read at start from data/ (RD27, settings.toml's
# version_facts and version_data)
COPY --chown=dainty:dainty crates/rustic-isles-1-14-7/data/version_facts.json data/1.14.7/
COPY --chown=dainty:dainty restricted/modules/rustic-isles/version-data/1.14.7/version_data.json data/1.14.7/
ARG ISLES=modules/rustic-isles/reference/1.14.7
COPY --chown=dainty:dainty restricted/${ISLES}/ParadoxBackend/game-data/ ${ISLES}/ParadoxBackend/game-data/
COPY --chown=dainty:dainty restricted/${ISLES}/ParadoxBackend/store-captures.json ${ISLES}/ParadoxBackend/
COPY --chown=dainty:dainty restricted/${ISLES}/ParadoxDirector/game-data/ ${ISLES}/ParadoxDirector/game-data/
COPY --from=rust-build /out/bin/ bin/
COPY --from=rust-build /out/plugins/ plugins/
COPY --chmod=0755 restricted/docker/director-entrypoint.sh bin/dainty-director
COPY --chmod=0755 restricted/docker/host-entrypoint.sh bin/dainty-host-docker
COPY --chmod=0755 restricted/docker/healthcheck.sh bin/dainty-healthcheck
# The network profile's one-shot (compose.yaml network-init, DN12)
COPY --chmod=0755 restricted/docker/network/init.sh bin/dainty-network-init
RUN mkdir -p .workspace "$ISLES/Items_Analysis" "$ISLES/DauntlessEndpointDocumentation" \
 && chown dainty:dainty .workspace "$ISLES/Items_Analysis" "$ISLES/DauntlessEndpointDocumentation"
# The commit the backend names to players (DAINTY_SOURCE_COMMIT, the AGPL source offer). setup reads it from
# git or release.json, neither of which is in the image; the process environment wins over the env file.
ARG DAINTY_COMMIT=""
ENV DAINTY_SOURCE_COMMIT=${DAINTY_COMMIT} \
    PATH=/opt/dainty-lass/bin:$PATH
USER dainty
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["dainty-host", "--help"]
LABEL org.opencontainers.image.title="Dainty Lass server" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.source="https://git.silentspud.com/silentspud/dainty-lass"

# ------------------------------------------------------------------ netem: latency for the network profile (DN12)
# tc and nothing else, joined to a node's network namespace with CAP_NET_ADMIN (compose.yaml, netem-*)
FROM ${DEBIAN_IMAGE} AS netem
RUN apt-get update && apt-get install -y --no-install-recommends iproute2 && rm -rf /var/lib/apt/lists/*
COPY --chmod=0755 restricted/docker/network/netem.sh /usr/local/bin/dainty-netem
ENTRYPOINT ["/usr/local/bin/dainty-netem"]
LABEL org.opencontainers.image.title="Dainty Lass network harness latency" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.source="https://git.silentspud.com/silentspud/dainty-lass"

# ------------------------------------------------------------------ test: every server suite on the server image
# The server image with the Rust toolchain, the workspace's sources and its tests compiled in, so a test run
# (docker/test-in-image.sh, `docker compose run --rm test`, docker/run-tests.sh) checks the code on the same OS,
# Wine and paths production runs on. Never deployed.
FROM server AS test
USER root
# procps: the host kit's start tests send Ctrl+C with kill(1), which the slim base images leave out. libdbus-1-dev:
# the gateway's tests take the launcher kit, whose keyring links D-Bus on Linux
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential pkg-config git nftables libcap2-bin procps libdbus-1-dev \
 && setcap cap_net_admin+ep /usr/sbin/nft \
 && rm -rf /var/lib/apt/lists/*
COPY --from=rust-build /usr/local/cargo /usr/local/cargo
COPY --from=rust-build /usr/local/rustup /usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo RUSTUP_HOME=/usr/local/rustup \
    PATH=/usr/local/cargo/bin:/opt/dainty-lass/bin:$PATH \
    CARGO_TERM_COLOR=never
# The sources laid out as the repository is: the server crates are the MIT root workspace's, restricted/ beside it
COPY --chown=dainty:dainty --from=rust-sources /src/ /opt/src/
COPY --chmod=0755 restricted/docker/test-in-image.sh bin/dainty-test
RUN chmod -R a+rwX /usr/local/cargo /usr/local/rustup
USER dainty
# The server crates' tests, built now so a run starts at once (the downloaded crates stay in the image: a run
# rebuilds nothing and needs no network). The launcher (Tauri, WebKitGTK) is not a server crate and is not
# built; the fuzz targets are a workspace of their own.
# The gateway library first: the host's controller tests load it (a cdylib is not built for its own tests)
RUN cd /opt/src && cargo build --locked -p dainty-gateway-plugin --lib && cargo test --locked --no-run \
      -p dainty-protocol -p dainty-proxy -p dainty-remote -p dainty-service-kit -p dainty-store -p dainty-gateway -p dainty-content -p dainty-admin -p dainty-hostkit -p dainty-host \
      -p dainty-controller
ENTRYPOINT ["/usr/bin/tini", "--", "dainty-test"]
CMD []

# ------------------------------------------------------------------ rollback: Node services and MongoDB tools (until RB15)
FROM ${NODE_IMAGE} AS node-build
WORKDIR /src
ARG ISLES=modules/rustic-isles/reference/1.14.7
COPY restricted/${ISLES}/ParadoxBackend/package.json restricted/${ISLES}/ParadoxBackend/package-lock.json ${ISLES}/ParadoxBackend/
COPY restricted/${ISLES}/ParadoxDirector/package.json restricted/${ISLES}/ParadoxDirector/package-lock.json ${ISLES}/ParadoxDirector/
RUN --mount=type=cache,id=dainty-npm,target=/root/.npm \
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
WORKDIR /opt/dainty-lass
COPY --chown=node:node restricted/LICENSE.txt restricted/NOTICE.md restricted/ADDITIONAL_TERMS.md restricted/README.md ./
COPY --chown=node:node native/ native/
COPY --chown=node:node restricted/Cargo.toml ./
COPY --chown=node:node restricted/modules/rustic-isles/ADDITIONAL_TERMS.md restricted/modules/rustic-isles/LICENSE restricted/modules/rustic-isles/NOTICE.md modules/rustic-isles/
COPY --chown=node:node crates/rustic-isles-1-14-7/data/version_facts.json data/1.14.7/
COPY --chown=node:node restricted/modules/rustic-isles/version-data/1.14.7/version_data.json data/1.14.7/
COPY --chown=node:node --from=node-build /src/modules/rustic-isles/reference/1.14.7/ modules/rustic-isles/reference/1.14.7/
COPY --from=rust-build /out/bin/ bin/
COPY --chmod=0755 restricted/docker/director-entrypoint.sh bin/dainty-director
COPY --chmod=0755 restricted/docker/host-entrypoint.sh bin/dainty-host-docker
COPY --chmod=0755 restricted/docker/healthcheck.sh bin/dainty-healthcheck
COPY --chmod=0755 restricted/docker/network/init.sh bin/dainty-network-init
ARG ISLES=modules/rustic-isles/reference/1.14.7
RUN mkdir -p .workspace "$ISLES/Items_Analysis" "$ISLES/DauntlessEndpointDocumentation" \
 && chown node:node .workspace "$ISLES/Items_Analysis" "$ISLES/DauntlessEndpointDocumentation"
ARG DAINTY_COMMIT=""
ENV DAINTY_SOURCE_COMMIT=${DAINTY_COMMIT} \
    MONGODB_URI=mongodb://127.0.0.1:27017/?replicaSet=rs0 \
    MONGODB_DB=reforged \
    MONGODB_APP_NAME=reforged-backend \
    MONGODB_TRANSACTION_MAX_COMMIT_TIME_MS=15000 \
    PATH=/opt/dainty-lass/bin:$PATH
USER node
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["dainty-host", "--help"]
LABEL org.opencontainers.image.title="Dainty Lass server (Node rollback)" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.source="https://git.silentspud.com/silentspud/dainty-lass"
