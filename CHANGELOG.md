# Changelog

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the versioning follows [Semantic Versioning](https://semver.org/lang/en/).

## [Unreleased]

## [0.1.0] - 2026-10-07

### Added

- `nvsn` CLI as a single binary for Linux, macOS, Windows and Android (Termux), with `--json` output (`schema_version` 1).
- Version commands: `install` (without an argument it reads `.nvmrc` or `.node-version` upwards), `uninstall`, `list`, `list-remote` (`--lts`, `--major`, `--latest`), `version`, `version-remote`, `which`, `current`, `default`.
- User aliases: `alias` and `unalias`.
- Running without changing the session: `run` and `exec`.
- Shell integration: `use`, `deactivate`, `env` and `init` (bash, zsh, fish and PowerShell 5.1 and 7). `init --apply` asks for confirmation unless `--yes` is passed.
- Directory-change hook that filters by `.nvmrc` or `.node-version` before calling the binary.
- Version specifications: exact, partial, `lts/*`, `lts/<codename>`, `lts/-N`, `latest` and `node`, and semver ranges. `stable`, `unstable` and `iojs` return an explicit error.
- Node.js index cache with `--offline` mode.
- `completions` for bash, zsh, fish and PowerShell.
- `doctor` to check the installation, the platform, the shell and the index.
- `self-update` (with `--check`) and `self-uninstall` (with confirmation, without touching anything outside `NVSN_DIR`).
- Installers `install.sh` and `install.ps1`, and their uninstallers, for Linux, macOS and Windows.
- Stable exit codes (0 to 10); see `nvsn --help` for the full table.
- Local performance benchmarks for `nvsn-core` and `nvsn-shell` (`cargo bench`); see the README's Benchmarks section.

### Security

- Mandatory SHA-256 checksum for every Node.js download, for `self-update` and for the installer. If it does not match or cannot be verified, nothing is installed.
- Mandatory HTTPS for the index, binaries, mirrors and redirects. `--allow-http` relaxes it explicitly and is insecure.
- Safe extraction of `.tar.gz` and `.zip`: rejects `..` and absolute paths, links that escape the destination, absolute links and writes through existing links.
- Extraction limits (2 GiB and 100,000 entries by default) against zip bombs.
- Atomic installation: extraction into a temporary directory and renamed at the end; per-version lock with a timeout.
- No `sudo` or elevation; nvsn does not modify the system `PATH` or shell profiles without `--apply`.

### Fixed

- `nvsn default` takes effect in login shells and graphical applications (login block that reads the default on every startup).
- `nvsn run` distinguishes a version that is in the index but not installed.
- `which`, `alias` and `current` use the same effective version (session and, if there is none, the default).
- Corrupt index, insecure URL and `NVSN_DIR` pointing to a file exit with the correct exit code.
- Error messages of `nvsn-core` in English.
- Busy lock on Windows (error 33) during the concurrent installation of a version.
- When leaving a directory with an automatic version, the version was not reverted in bash, zsh, fish and PowerShell.
- Resuming downloads with 416 responses and incorrect `Content-Range` headers.
- Chained symbolic links that escaped the destination during extraction.
- `run`/`exec` now forward a bare `--help`/`-h`/`--version`/`-V` to the child process when a version is given, instead of nvsn intercepting it; without a version, they show nvsn's own help/version for the subcommand instead of a usage error.
