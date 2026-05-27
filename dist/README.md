# Ax Release Artifacts

Ax releases publish one binary per supported OS/architecture plus `SHA256SUMS`.

| Platform | Artifact |
| --- | --- |
| macOS arm64 | `ax-darwin-arm64` |
| macOS x64 | `ax-darwin-x64` |
| Linux x64 | `ax-linux-x64` |
| Linux arm64 | `ax-linux-arm64` |
| Windows x64 | `ax-windows-x64.exe` |

## One-Line Install

```bash
curl -fsSL https://raw.githubusercontent.com/axlanguage/axlang/main/dist/install.sh | sh
```

```powershell
iwr https://raw.githubusercontent.com/axlanguage/axlang/main/dist/install.ps1 -useb | iex
```

The installers download from `https://github.com/axlanguage/axlang/releases/latest/download` by default. Set `AX_VERSION` to install a tagged release such as `v1.0.0`, set `AX_RELEASE_BASE` to use a mirror or local artifact directory, and set `AX_BIN_DIR` to choose the install directory.

Release binaries embed the standard pack manifests and runtime C sources, so
`ax packs`, `ax check`, `ax build`, and `ax run` work without a source checkout,
separate `std/` download, or separate runtime source bundle.

Windows binaries support the Ax CLI and basic native builds. TCP/HTTP network
runtime files compile with limited stubs on Windows; use macOS or Linux for full
native network execution in v1.0.

## Local Host Build

```bash
./dist/build-release.sh
```

This creates `dist/bin/ax-<host-target>` and a matching `dist/bin/SHA256SUMS` entry for the current machine.

Run the local release and installer smoke:

```bash
./dist/verify-release.sh
```

The local smoke installs the staged host artifact, checks `ax version` and
`ax packs`, runs `examples/agents/fs_journal.ax`, and runs
`examples/release_smoke.ax` with CLI arguments to cover standard packs on POSIX
hosts.

## Release Workflow

`.github/workflows/release.yml` builds the release matrix, verifies each binary
with `ax version`, builds and runs `examples/release_smoke.ax` on POSIX targets,
verifies Windows with basic `hello` and `primitives` native builds, creates
`SHA256SUMS`, uploads a combined workflow artifact, and attaches assets to
GitHub releases for `v*` tags.

Create a public release:

```bash
git tag v1.0.0
git push origin v1.0.0
gh release view v1.0.0 --repo axlanguage/axlang
```

The tag push runs the release workflow and uploads the platform binaries plus
`SHA256SUMS` to the GitHub Release.
