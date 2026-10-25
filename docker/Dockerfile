# syntax=docker/dockerfile:1
#
# DaintyLass server image (docs/hosting-docker.md). Built from the repository root:
#   docker compose build            (compose.yaml), or
#   docker build -f docker/Dockerfile --target server .
#
# One deployable image, laid out as the server release is (RD8, .github/workflows/server-release.yml), under
# /opt/reforged, so every path the server writes is the same in every container:
#   server  reforged-server-minimal, the whole server as one process (docs/app-structure-plan.md: the gateway, the
#           allowlist with nftables, the content server, the admin API and the in-process game stack), with
#           reforged-server-console for the host kit's commands, Wine for the game servers, and the Node backend and
#           Director sources the host kit's gamedata and the test image still use
#   test    the server image with the Rust toolchain and the server crates' tests compiled in; never deployed

ARG RUST_IMAGE=rust:1-slim-trixie
ARG NODE_IMAGE=node:22-trixie-slim

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
# The two server binaries (docs/app-structure-plan.md): the headless runner the server container runs, and the
# console one with the host kit's commands. The launcher, the GUI and client bins and the single-edition bins are
# not built.
RUN --mount=type=cache,id=reforged-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=reforged-cargo-target,target=/src/rust/target \
    cargo build --release --locked -p server-minimal -p server-console \
 && mkdir -p /out/bin \
 && cp target/release/reforged-server-minimal target/release/reforged-server-console /out/bin/

# ------------------------------------------------------------------ the Node sources: gamedata, the test suites
FROM ${NODE_IMAGE} AS node-build
WORKDIR /src
# Every dependency, dev ones included, as the server release has them: gamedata runs Mystic Paradox's
# generators, some with tsx. The lock files first, so a source change does not reinstall.
COPY paradox/backend/package.json paradox/backend/package-lock.json paradox/backend/
COPY paradox/director/package.json paradox/director/package-lock.json paradox/director/
RUN --mount=type=cache,id=reforged-npm,target=/root/.npm \
    (cd paradox/backend && npm ci --no-audit --no-fund) \
 && (cd paradox/director && npm ci --no-audit --no-fund)
COPY paradox/ paradox/
RUN (cd paradox/backend && npm run build) \
 && (cd paradox/director && npm run build) \
 && find . -type d -name .bin -path '*/node_modules/*' -prune -exec rm -rf {} +

# ------------------------------------------------------------------ server: the one process, the host kit's commands
FROM ${NODE_IMAGE} AS server
# Wine and a virtual display for the Windows game servers (docs/hosting-linux.md); tini reaps them. nftables for the
# allowlist: the server runs as the node user with CAP_NET_ADMIN only in its bounding set (compose.yaml), and nft gets
# it from its file capability, nothing else does. That capability needs privilege gain allowed in the container, so
# no setuid binary stays in the image: none may hand a compromised gateway root, and root the bounding set's NET_ADMIN.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates wine wine64 xvfb xauth tini nftables libcap2-bin \
 && setcap cap_net_admin+ep /usr/sbin/nft \
 && apt-get purge -y libcap2-bin && apt-get autoremove -y \
 && rm -rf /var/lib/apt/lists/* \
 && find / -xdev -perm /6000 -type f -exec chmod a-s {} +
# The node user (uid 1000) owns everything the server writes: the workspace and the game-data folders, which
# compose.yaml mounts as volumes (a new volume takes this ownership and these files).
WORKDIR /opt/reforged
COPY --chown=node:node LICENSE.txt NOTICE.md ADDITIONAL_TERMS.md README.md ./
COPY --chown=node:node native/ native/
COPY --chown=node:node --from=node-build /src/paradox/ paradox/
COPY --from=rust-build /out/bin/ bin/
COPY --chmod=0755 docker/server-entrypoint.sh bin/reforged-server
COPY --chmod=0755 docker/host-entrypoint.sh bin/reforged-host-docker
COPY --chmod=0755 docker/healthcheck.sh bin/reforged-healthcheck
RUN mkdir -p .workspace paradox/Items_Analysis paradox/DauntlessEndpointDocumentation \
 && chown node:node .workspace paradox/Items_Analysis paradox/DauntlessEndpointDocumentation
# The commit the server names to players (REFORGED_SOURCE_COMMIT, the AGPL source offer): neither git nor
# release.json is in the image, and the process environment wins over any file.
ARG REFORGED_COMMIT=""
ENV REFORGED_SOURCE_COMMIT=${REFORGED_COMMIT} \
    PATH=/opt/reforged/bin:$PATH
USER node
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["reforged-server"]
LABEL org.opencontainers.image.title="DaintyLass server" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.source="https://git.silentspud.com/silentspud/dainty-lass"

# ------------------------------------------------------------------ test: every server suite on the server image
# The server image with the Rust toolchain, the workspace's sources and its tests compiled in, so a test run
# (docker/test-in-image.sh, `docker compose run --rm test`, docker/run-tests.sh) checks the code on the same OS,
# Node, Wine and paths production runs on. Never deployed.
FROM server AS test
USER root
# procps: the host kit's start tests send Ctrl+C with kill(1), which the slim base images leave out
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential pkg-config git procps \
 && rm -rf /var/lib/apt/lists/*
COPY --from=rust-build /usr/local/cargo /usr/local/cargo
COPY --from=rust-build /usr/local/rustup /usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo RUSTUP_HOME=/usr/local/rustup \
    PATH=/usr/local/cargo/bin:/opt/reforged/bin:$PATH \
    CARGO_TERM_COLOR=never
COPY --chown=node:node rust/ rust/
COPY --chmod=0755 docker/test-in-image.sh bin/reforged-test
RUN chown node:node rust && chmod -R a+rwX /usr/local/cargo /usr/local/rustup
USER node
# The server crates' tests, built now so a run starts at once (the downloaded crates stay in the image: a run
# rebuilds nothing and needs no network). The launcher (Tauri, WebKitGTK) is not a server crate and is not
# built; the fuzz targets are a workspace of their own.
RUN cd rust && cargo test --locked --no-run \
      -p protocol -p proxy -p service-kit -p store -p gateway -p content -p admin -p host \
      -p controller -p backend -p director
ENTRYPOINT ["/usr/bin/tini", "--", "reforged-test"]
CMD []
