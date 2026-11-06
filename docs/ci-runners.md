# CI runners

dainty-lass runs its checks on Forgejo Actions at `git.silentspud.com` (DX3), for both workspaces: the MIT root
and the AGPL `restricted/` (RD27). The workflows are in the root `.forgejo/workflows/`; Forgejo reads only that
folder and ignores `.github/workflows/` once it exists, so the GitHub workflows Mystic Paradox and Dauntless
Revived came with stay as they were, for GitHub only.
This page is the maintainer's: the runners, their labels and how to register them.

## Labels

Three labels. One Linux machine can serve the first two from one runner.

| Label | Runs on | Used for |
| --- | --- | --- |
| `docker` | A job container from `node:22-bookworm` on a Linux runner | Rust (rustup is installed by the job), Node, Bun, cargo-deny, fuzzing, the change detection and `CI ok` |
| `linux-host` | The Linux runner itself (host mode) | restricted's Docker and network jobs: they build the images and run the compose smoke test on the runner's own Docker |
| `windows-msvc` | The maintainer's Windows machine (host mode) | MSVC builds: both Rust workspaces on Windows, restricted's unsigned NSIS installers (`installers-windows`), and the release builds on Windows |

| Workflow | Jobs | Labels |
| --- | --- | --- |
| `ci.yml` (every push) | The root's Rust, cargo-deny and fuzz; the UI; `restricted/`'s Rust, cargo-deny and fuzz, the Paradox reference, Docker, the installers and the network | `docker`, `linux-host`, `windows-msvc` |
| `server-release.yml`, `launcher-release.yml` (Run workflow, a dry run) | Build from `restricted/` | `docker`, `windows-msvc` |

rustic-isles' and rustic-rifts' own workflows (`ci.yml`, `routes.yml`, and RF3's `transport.yml` for the
transport DLL and its self-test) are still in `restricted/modules/<name>/.forgejo/workflows/` from before they moved
in-tree. Forgejo doesn't read a workflow there, so nothing runs them.

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
2. **A kernel older than 6.19**, for the Docker job: MongoDB 8.0 refuses to start on 6.19 (mongodb.org
   SERVER-121912), and the smoke test starts `mongo:8.0`. Debian 12 and Ubuntu 24.04 kernels are fine.
3. Register, as the runner's user, from the folder it will run in:

   ```sh
   forgejo-runner register --no-interactive \
     --instance https://git.silentspud.com \
     --token <registration token> \
     --name linux-1 \
     --labels 'docker:docker://node:22-bookworm,linux-host:host'
   forgejo-runner generate-config > config.yml
   ```

4. In `config.yml`: `runner.capacity: 2` or more (the Rust, fuzz and deny jobs run side by side) and
   `cache.enabled: true`. Leave `container.docker_host` as generated: the jobs in containers need no Docker of
   their own (cargo-deny's action is a Docker action, which the runner starts itself), and the Docker job runs on
   the host.
5. Run it as a service: `forgejo-runner daemon --config config.yml`, from a systemd unit with
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

## What isn't here

- **Publishing a release.** `.forgejo/workflows/server-release.yml` is a dry run on Forgejo: `dainty-host update` reads GitHub's
  releases API and checks GitHub attestations. The tag trigger and the publish job come back with DX5, from
  `b28fd27:.github/workflows/server-release.yml`.
- **Dependency updates.** `.github/dependabot.yml` is GitHub's and does nothing on Forgejo. Renovate is the
  Forgejo equivalent, if wanted before DX5.
- **Mystic's and DR's other jobs.** rustic-isles' launcher, runtime loader and gitleaks jobs stay in
  `restricted/modules/rustic-isles/.github/workflows/`, for GitHub only.
