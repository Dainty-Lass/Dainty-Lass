# syntax=docker/dockerfile:1
# Dainty Lass's images (RD27): MIT binaries from the root workspace. The only AGPL bytes are the data files the
# server image ships under data/ and the reference folders (docs/port/spec-docker.md 2.4).
#   front    gateway, allowlist, content server, admin API
#   server   the host kit, backend and Director, Wine for the game servers
#   netem    latency for the network profile's harness (DN12)
#   test     every server suite, never deployed
# Build from the repository root: docker compose build (compose.yaml), or
#   docker build -f docker/Dockerfile --target server .

ARG RUST_IMAGE=rust:1-slim-trixie
ARG DEBIAN_IMAGE=debian:trixie-slim

# What cargo reads: the root workspace alone (D-7)
FROM scratch AS rust-sources
COPY Cargo.toml Cargo.lock rust-toolchain.toml /src/
COPY .cargo/ /src/.cargo/
COPY crates/ /src/crates/
# The launcher kit's build reads its version
COPY launcher/package.json /src/launcher/package.json
# dainty-hostkit's tests check each seeded file has its committed example (AGPL data, copied by path, Q-2)
COPY restricted/modules/rustic-isles/reference/1.14.7/ParadoxBackend/game-data/ \
     /src/restricted/modules/rustic-isles/reference/1.14.7/ParadoxBackend/game-data/
COPY restricted/modules/rustic-isles/reference/1.14.7/ParadoxDirector/game-data/ \
     /src/restricted/modules/rustic-isles/reference/1.14.7/ParadoxDirector/game-data/

FROM ${RUST_IMAGE} AS rust-build
# mingw builds dainty-inject.exe, which loads the runtime into each game server inside Wine (RD9, 2026-11-13)
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential pkg-config ca-certificates git gcc-mingw-w64-x86-64 \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
# The pinned toolchain in its own layer, so a source change doesn't download it again
COPY rust-toolchain.toml ./
RUN (rustup show active-toolchain || rustup toolchain install) && rustup target add x86_64-pc-windows-gnu
COPY --from=rust-sources /src/ /src/
ARG DAINTY_MODULES=all-modules
RUN --mount=type=cache,id=dainty-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=dainty-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=dainty-cargo-target,target=/src/target \
    set -eu; \
    set --; \
    for f in $(echo "$DAINTY_MODULES" | tr , ' '); do set -- "$@" --features "dainty-server/$f"; done; \
    cargo build --release --locked -p dainty-server "$@" --bins; \
    cargo build --release --locked -p dainty-legacy --bin dainty-legacy; \
    cargo build --release --locked -p dainty-inject --target x86_64-pc-windows-gnu; \
    cargo build --release --locked -p rustic-isles-1-14-7-plugin -p dainty-content-plugin -p dainty-chat-plugin \
      -p dainty-gateway-plugin --lib; \
    mkdir -p /out/bin /out/plugins; \
    cp target/release/dainty-host target/release/dainty-legacy /out/bin/; \
    cp target/x86_64-pc-windows-gnu/release/dainty-inject.exe /out/bin/; \
    for lib in rustic_isles_1_14_7_plugin dainty_content_plugin dainty_chat_plugin dainty_gateway_plugin; do \
      cp "target/release/lib$lib.so" /out/plugins/; \
    done; \
    cp /out/plugins/librustic_isles_1_14_7_plugin.so /out/plugins/librustic_isles_plugin.so
# librustic_isles_plugin.so: the pre-rename name, so existing settings.toml pins keep loading for one release (Q-3)

FROM ${DEBIAN_IMAGE} AS front
# nft is the only file with a capability: the unprivileged allowlist gets CAP_NET_ADMIN through it alone
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates nftables libcap2-bin \
 && setcap cap_net_admin+ep /usr/sbin/nft \
 && apt-get purge -y libcap2-bin && apt-get autoremove -y \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --gid 1000 dainty \
 && useradd --uid 1000 --gid 1000 --home-dir /opt/dainty-lass --no-create-home --shell /usr/sbin/nologin dainty \
 && install -d -o dainty -g dainty /opt/dainty-lass/.workspace
COPY --from=rust-build /out/bin/dainty-host /opt/dainty-lass/bin/
COPY --chmod=0755 docker/healthcheck.sh /opt/dainty-lass/bin/dainty-healthcheck
ENV PATH=/opt/dainty-lass/bin:$PATH
WORKDIR /opt/dainty-lass/.workspace
USER dainty
LABEL org.opencontainers.image.title="Dainty Lass front door" \
      org.opencontainers.image.source="https://git.silentspud.com/silentspud/dainty-lass" \
      org.opencontainers.image.licenses="MIT"

FROM ${DEBIAN_IMAGE} AS server
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates wine wine64 xvfb xauth tini \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --gid 1000 dainty \
 && useradd --uid 1000 --gid 1000 --home-dir /opt/dainty-lass --no-create-home --shell /usr/sbin/nologin dainty
WORKDIR /opt/dainty-lass
ARG ISLES=modules/rustic-isles/reference/1.14.7
# Named volumes take these folders' ownership when first made, so dainty owns every one
RUN install -d -o dainty -g dainty /opt/dainty-lass/modules /opt/dainty-lass/modules/rustic-isles \
      "$ISLES" "$ISLES/ParadoxBackend" "$ISLES/ParadoxBackend/game-data" "$ISLES/ParadoxDirector" \
      "$ISLES/ParadoxDirector/game-data" "$ISLES/Items_Analysis" "$ISLES/DauntlessEndpointDocumentation" .workspace
# The host kit's project-root marker: Cargo.toml beside modules/rustic-isles/, only its presence read (D-3)
COPY --chown=dainty:dainty Cargo.toml ./
COPY --chown=dainty:dainty \
     restricted/modules/rustic-isles/reference/1.14.7/ParadoxBackend/game-data/*.example.json \
     $ISLES/ParadoxBackend/game-data/
COPY --chown=dainty:dainty \
     restricted/modules/rustic-isles/reference/1.14.7/ParadoxDirector/game-data/*.example.json \
     $ISLES/ParadoxDirector/game-data/
COPY --chown=dainty:dainty restricted/modules/rustic-isles/reference/1.14.7/ParadoxBackend/store-captures.json \
     $ISLES/ParadoxBackend/
# The 1.14.7 version facts (MIT) and Mystic Paradox's design beside them, read at start; root's and read-only,
# never a host's file (D-10, RD27, port plan item 9)
COPY crates/rustic-isles-1-14-7/data/version_facts.json data/1.14.7/
COPY restricted/modules/rustic-isles/version-data/1.14.7/version_data.json data/1.14.7/
# The AGPL data's licence and notices beside it (D-4, Q-1)
COPY restricted/modules/rustic-isles/LICENSE restricted/modules/rustic-isles/NOTICE.md \
     restricted/modules/rustic-isles/ADDITIONAL_TERMS.md data/
COPY restricted/NOTICE.md restricted/ADDITIONAL_TERMS.md data/restricted/
COPY LICENSE.txt ./
# A Windows build context marks every file executable
RUN find data -type d -exec chmod 0755 {} + \
 && find data LICENSE.txt Cargo.toml modules -type f -exec chmod 0644 {} +
COPY --from=rust-build /out/bin/ bin/
COPY --from=rust-build /out/plugins/ plugins/
COPY --chmod=0755 docker/director-entrypoint.sh bin/dainty-director
COPY --chmod=0755 docker/host-entrypoint.sh bin/dainty-host-docker
COPY --chmod=0755 docker/healthcheck.sh bin/dainty-healthcheck
COPY --chmod=0755 docker/network/init.sh bin/dainty-network-init
ARG DAINTY_COMMIT=
# setup finds no git or release.json here; the environment wins over the env file, so players see this (D-5)
ENV DAINTY_SOURCE_COMMIT=${DAINTY_COMMIT} \
    PATH=/opt/dainty-lass/bin:$PATH
USER dainty
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["dainty-host", "--help"]
LABEL org.opencontainers.image.title="Dainty Lass server" \
      org.opencontainers.image.source="https://git.silentspud.com/silentspud/dainty-lass" \
      org.opencontainers.image.licenses="MIT AND AGPL-3.0-only"

FROM ${DEBIAN_IMAGE} AS netem
RUN apt-get update \
 && apt-get install -y --no-install-recommends iproute2 \
 && rm -rf /var/lib/apt/lists/*
COPY --chmod=0755 docker/network/netem.sh /usr/local/bin/dainty-netem
ENTRYPOINT ["/usr/local/bin/dainty-netem"]
LABEL org.opencontainers.image.title="Dainty Lass network harness latency" \
      org.opencontainers.image.source="https://git.silentspud.com/silentspud/dainty-lass" \
      org.opencontainers.image.licenses="MIT"

FROM server AS test
USER root
# procps: the start tests stop it with kill(1); libdbus: the gateway's tests take the launcher kit's keyring
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential pkg-config git nftables libcap2-bin procps \
      libdbus-1-dev \
 && setcap cap_net_admin+ep /usr/sbin/nft \
 && rm -rf /var/lib/apt/lists/*
COPY --from=rust-build /usr/local/cargo /usr/local/cargo
COPY --from=rust-build /usr/local/rustup /usr/local/rustup
RUN chmod -R a+rwX /usr/local/cargo /usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo \
    RUSTUP_HOME=/usr/local/rustup \
    PATH=/usr/local/cargo/bin:/opt/dainty-lass/bin:$PATH \
    CARGO_TERM_COLOR=never
COPY --from=rust-sources --chown=dainty:dainty /src/ /opt/src/
COPY --chmod=0755 docker/test-in-image.sh /opt/dainty-lass/bin/dainty-test
USER dainty
# Built now, so a run needs no network; the controller's tests load the gateway library, which they don't build
RUN cd /opt/src \
 && cargo build --locked -p dainty-gateway-plugin --lib \
 && cargo test --locked --no-run -p dainty-protocol -p dainty-proxy -p dainty-remote -p dainty-service-kit \
      -p dainty-store -p dainty-gateway -p dainty-content -p dainty-admin -p dainty-hostkit -p dainty-host \
      -p dainty-controller -p dainty-server
ENTRYPOINT ["/usr/bin/tini", "--", "dainty-test"]
CMD []
LABEL org.opencontainers.image.title="Dainty Lass server tests" \
      org.opencontainers.image.licenses="MIT"
