# CI runners

dainty-lass runs its checks on Forgejo Actions at `git.silentspud.com` (DX3), for both workspaces: the MIT root
and the AGPL `restricted/` (RD27). The workflows are in the root `.forgejo/workflows/`; Forgejo reads only that
folder and ignores `.github/workflows/` once it exists. `.github/workflows/` holds GitHub's copies of the three
(RR3), which GitHub runs on its hosted runners and Forgejo ignores; see [GitHub Actions](#github-actions). A change
to one copy is made to the other in the same commit.
This page is the maintainer's: the runners, their labels and how to register them.

## Labels

Three labels. One Linux machine can serve the first two from one runner.

| Label | Runs on | Used for |
| --- | --- | --- |
| `docker` | A job container from `node:22-bookworm` on a Linux runner | Rust (rustup is installed by the job), Node, Bun, cargo-deny, fuzzing, the change detection and `CI ok` |
| `linux-host` | The Linux runner itself (host mode) | The Docker and network jobs: they build the images and run the compose smoke test on the runner's own Docker |
| `windows-msvc` | The maintainer's Windows machine (host mode) | MSVC builds: the Rust workspace and the native crates on Windows, the unsigned NSIS installers (`installers-windows`), and the release builds on Windows |

| Workflow | Jobs | Labels |
| --- | --- | --- |
| `ci.yml` (every push) | The Rust workspace, cargo-deny and fuzz; the UI; Docker, the installers, the network and the native crates | `docker`, `linux-host`, `windows-msvc` |
| `server-release.yml`, `launcher-release.yml` (Run workflow, a dry run) | Build from the root workspace; the server archive takes `restricted/`'s files | `docker`, `windows-msvc` |

Register the runners on the **`silentspud` account** (a user, not an organisation), not on one repository, so every
repository reaches them: user settings, Actions, Runners, "Create new runner" shows the registration token. A job whose label
no runner carries waits in the queue; it doesn't fail.

## What every runner needs

- **forgejo-runner**, a current release from `code.forgejo.org/forgejo/runner/releases`, for the machine's
  platform.
- **Node.js** wherever an action runs: the actions (`checkout`, `setup-node`, `rust-cache`, the artifact actions)
  are JavaScript and run on the `node` the job finds. The `docker` label's image has it; a host-mode runner needs
  Node 22 on its `PATH`.
- **Outbound HTTPS** to `git.silentspud.com`, `data.forgejo.org` and `code.forgejo.org` (actions), `github.com` and
  `objects.githubusercontent.com` (actions named by URL, Bun, libjuice), `static.rust-lang.org` and `sh.rustup.rs`,
  `index.crates.io` and `static.crates.io`, `registry.npmjs.org` and `nodejs.org`.
- **The cache**: leave `cache.enabled: true` in the runner's config (the default). `rust-cache` and `setup-bun`
  store through it.

Actions under `actions/` (`checkout`, `setup-node`) are named without a host and resolve through the instance's
`[actions] DEFAULT_ACTIONS_URL`; `data.forgejo.org` mirrors both at the pinned commits. Every other action is named
by its full URL, so the setting doesn't matter for them.

## The Linux runner (`docker` and `linux-host`)

1. Docker Engine with the compose plugin, Node.js 22, git and bash. A user of its own for the runner, in the
   `docker` group.
2. Register, as the runner's user, from the folder it will run in:

   ```sh
   forgejo-runner register --no-interactive \
     --instance https://git.silentspud.com \
     --token <registration token> \
     --name linux-1 \
     --labels 'docker:docker://node:22-bookworm,linux-host:host'
   forgejo-runner generate-config > config.yml
   ```

3. In `config.yml`: `runner.capacity: 2` or more (the Rust, fuzz and deny jobs run side by side) and
   `cache.enabled: true`. Leave `container.docker_host` as generated: the jobs in containers need no Docker of
   their own (cargo-deny's action is a Docker action, which the runner starts itself), and the Docker job runs on
   the host.
4. Run it as a service: `forgejo-runner daemon --config config.yml`, from a systemd unit with
   `User=<runner user>`, `WorkingDirectory=` the folder above and `Restart=on-failure`.

Pin the image by digest if you want it fixed (`node:22-bookworm@sha256:...`); the label is the only place it is
named.

## The Windows runner (`windows-msvc`)

1. **Visual Studio 2026 or its Build Tools**, workload "Desktop development with C++", with the **v145** toolset
   (rustic-rifts' projects name it) and a **Windows 11 SDK** (10.0.26100 was checked). Rust's MSVC target links
   with the same tools.
2. **rustup** (`rustup-init.exe`, default host `x86_64-pc-windows-msvc`). The jobs install each repository's
   pinned toolchain from its `rust-toolchain.toml`.
3. **Git for Windows**, with its `bin` folder (`C:\Program Files\Git\bin`) on the runner's `PATH` **before**
   `C:\Windows\System32`: the workflows run `shell: bash`, and with WSL installed `System32\bash.exe` would answer
   instead.
4. **Long paths**: `git config --system core.longpaths true`, and Windows' own long paths on
   (`HKLM\SYSTEM\CurrentControlSet\Control\FileSystem`, `LongPathsEnabled` = 1, then restart). A deep checkout
   fails without them (DF1). The jobs also set `core.longpaths` for the runner's
   user before every checkout.
5. **Node.js 22**, **PowerShell 7** (`pwsh`, the launcher release) and **7-Zip** with `7z.exe` on the `PATH` (the
   release build zips the Windows archive with it).
6. A short work folder: in `config.yml`, `host.workdir_parent: C:\fr` or similar, so paths stay short.
7. Register, from the folder the runner will run in:

   ```powershell
   forgejo-runner register --no-interactive `
     --instance https://git.silentspud.com `
     --token <registration token> `
     --name windows-msvc-1 `
     --labels 'windows-msvc:host'
   forgejo-runner generate-config > config.yml
   ```

8. Run `forgejo-runner daemon --config config.yml` as a service or a scheduled task at startup, as a user that
   isn't an administrator. Host-mode jobs run as that user, with its files: keep nothing on that account that a
   job shouldn't read.

## Secrets and settings

Set them on the `silentspud` account (settings, Actions, Secrets), so every repository sees them. It is optional
today.

| Secret | Used by | When it is needed |
| --- | --- | --- |
| `CI_READ_TOKEN` | Nothing since `restricted/` moved in-tree (2026-11-05): restricted takes the root's crates by path and the UI job reads `restricted/` from the same checkout. Kept for a job that has to read another repository | When the job's own token can't read the other repository while the instance needs sign-in (DF1's finding). A token with read access to repositories |

Actions must be on for each repository (repository settings, Units, Actions) and on the instance
(`[actions] ENABLED = true` in `app.ini`, the default).

## Branch protection

`CI ok` is the one check to require on `plan` and `main`; the jobs it sums up are skipped
when their family didn't change. In Forgejo's branch protection, "Enable status check", the pattern is
`CI / CI ok*`.

## First run

1. On the repository's Actions tab, check that the workflows are listed.
2. Run CI with "Run workflow" on `plan`: every family runs, so every label is exercised.
3. Run the Server release by hand once. The release's artifact `server-release` holds the two
   archives and `SHA256SUMS.txt`.

## GitHub Actions

`.github/workflows/` runs on the `github` mirror, which the maintainer pushes by hand. `ci.yml` has the same families,
jobs and `CI ok` as Forgejo's, with the labels mapped to hosted runners (`docker` and `linux-host` to
`ubuntu-latest`, `windows-msvc` to `windows-latest`) and these differences:

- Postgres is the Ubuntu image's own, started rather than installed.
- Caches are saved from `main` and `plan` only, since GitHub keeps 10 GB a repository; other branches restore them.
- The root's fuzz targets are split across four jobs (`SHARDS`, `SHARD` in `scripts/ci/fuzz.sh`).
- A `scan` job runs Trivy over the tree on every run and fails only on a secret; the Docker job scans its images
  too, without failing. The Trivy image is pinned in `.github/trivy/Dockerfile`, which Dependabot keeps current.
- `audit.yml`: zizmor on the workflows, a weekly Trivy run, and dependency review on pull requests.

Repository variable `CODE_SCANNING` set to `true` (once the repository is public, or has Advanced Security) sends
Trivy's and zizmor's findings to code scanning and turns on dependency review. Unset, the findings stay in the log.

`server-release.yml` and `launcher-release.yml` build what Forgejo's do, plus a CycloneDX SBOM of each archive (and
of the launcher's dependencies) as a separate artifact. Their publish jobs, with build provenance and SBOM
attestations (`gh attestation verify <file> --repo SilentSpud/DaintyLass`), are written but unreachable until DX5:
the server's tag trigger is commented out and the launcher's prepare never sets `publish`.

## What isn't here

- **Publishing a release.** Both forges' release workflows are dry runs: `dainty-host update` reads GitHub's releases
  API and checks GitHub attestations, which Forgejo has neither of, and GitHub's publish jobs wait for DX5.
- **Dependency updates.** `.github/dependabot.yml` is GitHub's and does nothing on Forgejo. Renovate is the
  Forgejo equivalent, if wanted before DX5.
