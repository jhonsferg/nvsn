<div align="center">

# nvsn - Node Version Manager

**A fast, cross-platform Node.js version manager written in Rust.**
Install, switch, and pin any Node.js release - no `sudo`, no system dependencies, no fuss.

[![License](https://img.shields.io/badge/License-MIT-blue?style=for-the-badge&logo=opensourceinitiative&logoColor=white)](LICENSE)

[![Rust](https://img.shields.io/badge/Built_with-Rust-orange?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![TLS](https://img.shields.io/badge/TLS-rustls_%28no_OpenSSL%29-lightgrey?style=for-the-badge&logo=letsencrypt&logoColor=white)](https://github.com/rustls/rustls)

[![Windows](https://img.shields.io/badge/Windows-0078D4?style=for-the-badge&logo=windows11&logoColor=white)](#-status-and-limitations)
[![Linux](https://img.shields.io/badge/Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black)](#-status-and-limitations)
[![macOS](https://img.shields.io/badge/macOS-000000?style=for-the-badge&logo=apple&logoColor=white)](#-status-and-limitations)
[![Android](https://img.shields.io/badge/Android_%28Termux%29-3DDC84?style=for-the-badge&logo=android&logoColor=white)](#-status-and-limitations)

</div>

---

## ✨ What makes nvsn different?

nvsn is a Node.js version manager built from scratch in Rust, by the same author as [gvsn](https://github.com/jhonsferg/gvsn) (the Go version manager). It was designed with a single goal: work everywhere, require nothing.

- **No Node.js required** - you don't need Node.js installed to install Node.js. nvsn downloads the official distributions directly from nodejs.org.
- **No `sudo`, no root** - everything lives in a data directory under your user account (`%LOCALAPPDATA%\nvsn` on Windows, `~/.local/share/nvsn` on Linux and macOS).
- **A single binary** - one file (`nvsn` or `nvsn.exe`) and nothing else. The Windows build links the C runtime statically, so it does not need the Visual C++ Redistributable.
- **Cross-platform by design** - one codebase and one command set for Windows, Linux, macOS and Android (Termux) targets. Platform differences are isolated in one crate.
- **Mandatory SHA-256 verification** - every archive is checked against nodejs.org's `SHASUMS256.txt` before extraction. There is no flag or variable that skips it.
- **Safe extraction** - archive entries with `..`, absolute paths or links that escape the destination are rejected, and extraction is limited in size and number of entries.
- **Atomic installs** - a version is extracted into a temporary directory and renamed into place only when it is complete. An interrupted install never leaves a half-finished version behind.
- **Version files** - reads `.nvmrc` and `.node-version`, searching upwards from the current directory.
- **Session-scoped activation** - `nvsn use` changes the version of the current terminal only, and `nvsn run` / `nvsn exec` run a version without changing the session at all.
- **Global default for every application** - `nvsn default` also changes what login shells and graphical applications (editors and similar) see.
- **Explicit setup** - `nvsn init <shell> --apply` writes the shell integration after asking for confirmation. Nothing edits your profile or your PATH silently.
- **Scriptable** - `--json` output with a `schema_version` on most commands, and stable exit codes.
- **Clean removal** - `nvsn implode` and `nvsn self-uninstall` remove what nvsn manages.

### 🦀 Why Rust?

A fair question for a tool that manages Node.js. A few honest reasons, not a verdict on other languages:

- **Memory safety for the riskiest operations.** `nvsn self-update` replaces its own running binary, `nvsn init --apply` writes to your shell profiles and `nvsn implode` deletes files across your home directory. These are exactly the operations where a memory bug turns into a corrupted install or a broken shell. Rust's ownership model rules out that class of bug at compile time.
- **One codebase, static binaries.** The release matrix covers Linux (musl), macOS, Windows and Android targets from one repository, with cross-compilation handled by `cross`.
- **Independence from the ecosystem it manages.** nvsn is a standalone binary. Node.js is a runtime dependency of the Node.js tools it installs, not of nvsn itself.

Plenty of excellent Node.js version managers exist. Choosing Rust for this one was a deliberate decision for this project, not a claim that Rust is the right call for every CLI tool.

---

## 🚀 Features

- 📥 **Install any Node.js version** - by exact version, partial version, LTS line, LTS codename or semver range
- 🔐 **SHA-256 verification** - every download is checked against `SHASUMS256.txt`
- 📌 **Per-project pinning** - `.nvmrc` and `.node-version` files, with automatic switching from the shell hook
- 🐚 **Session-scoped activation** - `nvsn use` switches the version of the current terminal only
- 🌍 **Global default** - `nvsn default` applies to new shells and to graphical applications
- ⚡ **`nvsn run` and `nvsn exec`** - run any installed version without changing your session
- 🏷️ **Aliases** - `nvsn alias` gives a name to an installed version
- 📈 **`nvsn outdated`** - see which installed versions have a newer patch release
- 🧹 **`nvsn prune`** - remove installed versions that nothing references
- 🩺 **`nvsn doctor`** - diagnoses the installation, shell integration, PATH and index, with actionable hints
- 📡 **Offline mode** - `--offline` uses the cached index and never touches the network
- 🪞 **Mirrors** - `--mirror <URL>`, HTTPS only
- 🏁 **Shell completions** - Bash, Zsh, Fish and PowerShell
- 🧾 **Machine-readable output** - `--json` with a stable `schema_version`
- 🔄 **`nvsn self-update`** - update from GitHub Releases, with the checksum always verified
- 💣 **`nvsn implode`** - remove nvsn and everything it manages

---

## 📦 Installation

### 🪟 Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/jhonsferg/nvsn/main/install/install.ps1 | iex
```

> Installs `nvsn.exe` to `~\.local\bin`. The archive's checksum is verified against `checksums.txt` before extraction, and no administrator rights are needed. The installer does not change your PATH or your profile: if `~\.local\bin` is not in your PATH, it tells you so. Then enable the shell integration with `nvsn init powershell --apply` and open a new terminal.

### 🐧 Linux and 🍎 macOS

```sh
curl -fsSL https://raw.githubusercontent.com/jhonsferg/nvsn/main/install/install.sh | sh
```

> Installs `nvsn` to `~/.local/bin`. The archive's checksum is verified against `checksums.txt` before the binary is installed, and the installer does not edit your shell profile. Then enable the shell integration with `nvsn init bash --apply` (or `zsh` or `fish`) and open a new terminal.

### 📂 Custom install directory

```powershell
# 🪟 Windows
$env:NVSN_INSTALL_DIR = "C:\tools\nvsn"; irm https://raw.githubusercontent.com/jhonsferg/nvsn/main/install/install.ps1 | iex
```

```sh
# 🐧 Linux / 🍎 macOS
NVSN_INSTALL_DIR=~/.bin curl -fsSL https://raw.githubusercontent.com/jhonsferg/nvsn/main/install/install.sh | sh
```

> The data directory is separate from the binary directory. To change it, pass `--dir <PATH>` or set `NVSN_DIR` (see Configuration below).

### ✅ Verify the installation

```sh
nvsn --version
nvsn doctor
```

> `nvsn doctor` exits with code `1` if a check fails. Warnings (for example "no versions installed yet") do not change the exit code. Until you run `nvsn init`, doctor reports the shell integration as missing.

---

## 🗑️ Uninstallation

Two equivalent ways to remove nvsn completely - the installed Node.js versions, the data directory, the binary, and the nvsn blocks in your shell profiles:

**Option 1 - `nvsn implode`** (if the binary still works):

```sh
nvsn implode           # shows a summary and asks for confirmation
nvsn implode --force   # removes everything without asking
```

**Option 2 - standalone uninstaller script** (works even if the binary is broken or already gone):

```powershell
# 🪟 Windows
irm https://raw.githubusercontent.com/jhonsferg/nvsn/main/install/uninstall.ps1 | iex
```

```sh
# 🐧 Linux / 🍎 macOS
curl -fsSL https://raw.githubusercontent.com/jhonsferg/nvsn/main/install/uninstall.sh | sh
```

Both scripts show what will be removed and ask for confirmation first. Use `--dry-run` to preview with nothing deleted, or `--force` to skip the prompt. A piped POSIX script cannot prompt interactively, so pass the flags through (`curl ... | sh -s -- --dry-run`) or set `NVSN_UNINSTALL_DRY_RUN=1` / `NVSN_UNINSTALL_FORCE=1` before the command. On Windows, set `$env:NVSN_UNINSTALL_DRY_RUN = "1"` or `$env:NVSN_UNINSTALL_FORCE = "1"` first.

> 💡 If you customized `NVSN_DIR` or `NVSN_INSTALL_DIR` at install time, set the same variables before uninstalling so the script cleans the right locations.

---

## ⚡ Quick Start

```sh
# 🐚 1. Enable shell integration (asks before writing to your profile)
nvsn init bash --apply

# 📥 2. Install the latest Node.js 24 release
nvsn install 24

# 🌍 3. Make it the default for new shells and graphical applications
nvsn default 24

# 🔍 4. Check the version active in this shell
nvsn current

# 📌 5. Pin a version for the current project (writes .nvmrc)
nvsn local 24

# ⚡ 6. Run a command with a given version, without changing the default
nvsn exec 24 npm --version
```

---

## 📖 Commands

Every command accepts these global flags:

| Flag | Description |
| ---- | ----------- |
| `--json` | Machine-readable JSON on stdout (`schema_version` 1). |
| `-q`, `--quiet` | Suppress status messages (results are still printed). |
| `-v`, `--verbose` | Show HTTP request and response details on stderr. |
| `--no-color` | Disable colors. The `NO_COLOR` variable is also honored. |
| `--no-progress` | Disable the download progress indicator. |
| `--offline` | Do not touch the network; use only the cached index. |
| `--dir <PATH>` | Root directory of nvsn (env: `NVSN_DIR`). |
| `--mirror <URL>` | Node.js distribution mirror (env: `NVSN_NODEJS_ORG_MIRROR`). HTTPS only. |
| `--allow-http` | Permit plain `http://` URLs. Insecure: checksums come from the same host. |
| `-y`, `--yes` | Answer yes to confirmations. Required when there is no terminal. |

### 🎯 Choosing how to activate a version

nvsn offers four ways to use a version, each with a different scope:

| Command | Scope | Persisted where |
| ------- | ----- | --------------- |
| `nvsn use` | This terminal session only | Not persisted - lives only in the shell's environment |
| `nvsn default` | Every new shell, login shell and graphical application | `<NVSN_DIR>/default` and the `current` link |
| `nvsn local` | This project (and anyone who clones it) | `.nvmrc` in the project directory |
| `nvsn run` / `nvsn exec` | One command only | Nothing - the session is not changed |

---

### 📥 `nvsn install [version]`

Downloads a Node.js release from nodejs.org, verifies its SHA-256 and installs it. Alias: `i`. Without a version it reads `.nvmrc` or `.node-version`, searching upwards from the current directory.

```sh
nvsn install 24            # 🔢 latest 24.x release
nvsn install 24.21.0       # 🎯 exact version
nvsn install lts/*         # 🛟 latest LTS line
nvsn install lts/iron      # 🏷️ LTS line by codename
nvsn install "^20"         # 📐 semver range
nvsn install               # 📄 version from .nvmrc / .node-version
```

> 💡 If the version is already installed, nvsn says so and does nothing else. If the download is interrupted, re-running the command starts again without leaving a half-finished version.

---

### 🗑️ `nvsn uninstall <version>`

Removes an installed version. Alias: `rm`. The version can also be an alias.

```sh
nvsn uninstall 20.11.1
nvsn uninstall 20.11.1 --yes   # 🔇 no prompt (required without a terminal)
```

> ⚠️ If the version is the default, nvsn asks for confirmation first.

---

### 🔁 `nvsn use [version]`

Activates an installed version in the **current** shell. It needs the shell integration (`nvsn init`, see [Shell Integration](#-shell-integration)). Without it, `nvsn use` only reports what is needed. Without a version it reads `.nvmrc` upwards.

```sh
nvsn use 24
nvsn use                   # 📄 uses the version from .nvmrc
```

> 💡 The change applies to this terminal only. New terminals start with the default version.

---

### 🌍 `nvsn default <version>`

Sets the default version. It must already be installed. The default is what new shells, login shells and graphical applications (editors, for example) see.

```sh
nvsn default 24
```

> 💡 The `current` link inside the data directory always points to the default version (a junction on Windows, a symlink on Unix). Changing the default does not require rewriting your profile.

---

### 📌 `nvsn local <version>`

Writes a `.nvmrc` file in the current directory with the given version, as plain text.

```sh
nvsn local 24
nvsn local latest          # 🆕 writes the word "latest": the pin follows the newest release
```

> ⚠️ If the version is not installed yet, nvsn warns you and writes the file anyway.

---

### 🏷️ `nvsn alias [name] [version]` · `nvsn unalias <name>`

Creates, lists or removes aliases. An alias points to an installed version. Without arguments, `nvsn alias` lists them.

```sh
nvsn alias myal 24         # 🏷️ create
nvsn alias                 # 📋 list
nvsn unalias myal          # 🗑️ remove
```

```
myal -> v24.21.0
```

---

### ⚡ `nvsn run <version> [args...]`

Runs the `node` binary of an installed version with the given arguments. It does not change your session. Use `--` before arguments that start with a dash and clash with nvsn flags.

```sh
nvsn run 24 --version
nvsn run 24 -p "1+1"
```

```
2
```

---

### ⚡ `nvsn exec <version> <command> [args...]`

Runs any command with the `bin` directory of an installed version at the front of `PATH`, without changing the session or the default.

```sh
nvsn exec 24 node --version
nvsn exec 24 npm --version
```

---

### 📋 `nvsn list` (alias `ls`)

Lists the installed versions. The default one is marked.

```sh
nvsn list
```

```
   v24.21.0 (default)
```

---

### 🌐 `nvsn list-remote` (alias `ls-remote`)

Lists the versions available in the nodejs.org index.

| Flag | Description |
| ---- | ----------- |
| `--lts` | Only LTS releases. |
| `--major <MAJOR>` | Only one major line. |
| `--latest` | Only the most recent release of the selection. |

```sh
nvsn list-remote --lts --major 20 --latest --offline
```

```
v20.20.2     Iron       2026-03-24
```

> 💡 Without a cached index, `--offline` fails with exit code 5 and suggests running once without `--offline`.

---

### 📈 `nvsn outdated`

Checks every installed version against the index and reports whether a newer patch release exists in the same major.minor line. It refreshes the index if the cache has expired (6 hours) but does not download any binaries.

```sh
nvsn outdated
```

```
  Installed      Latest patch   Status
  v24.21.0       v24.21.0       up to date
```

---

### 🔍 `nvsn current`

Prints the version in effect: the one of this session if there is one, otherwise the default.

```sh
nvsn current
```

```
v24.21.0
```

If there is no active version it prints `none` and exits with code 0.

---

### 📂 `nvsn path [version]` · `nvsn which [version]`

`nvsn path` prints the `bin` directory of the active (or given) version, useful for scripting. `nvsn which` prints the path of its `node` binary.

```sh
nvsn path
nvsn which 24
```

---

### 🔢 `nvsn version <spec>` · `nvsn version-remote <spec>`

`nvsn version` prints the **installed** version that matches a specification. `nvsn version-remote` prints the version that a specification resolves to in the index, without installing anything.

```sh
nvsn version 24
nvsn version-remote lts/*
```

> 💡 The output of `version-remote` changes when Node.js publishes new releases. See [Version Syntax](#-version-syntax).

---

### 🔧 `nvsn env [--shell <shell>]`

Prints the shell script that configures a session (the `NVSN_*` variables, `PATH` and, with the integration, the hook). You normally do not run it by hand: the block written by `nvsn init` evaluates it for you.

| Flag | Description |
| ---- | ----------- |
| `--shell <SHELL>` | `bash`, `zsh`, `fish` or `powershell`. Detected when omitted. |

```sh
eval "$(nvsn env --shell bash)"
```

```powershell
# 🪟 PowerShell
nvsn env --shell powershell | Out-String | Invoke-Expression
```

---

### 🐚 `nvsn init <shell> [--apply]`

Shows, or with `--apply` writes, the shell integration block for `bash`, `zsh`, `fish` or `powershell`. Without `--apply` it only prints the block and the profile path. With `--apply` it asks for confirmation (pass `--yes` in scripts).

| Flag | Description |
| ---- | ----------- |
| `--apply` | Write the block into the shell profile. Asks unless `--yes` is given. |

```sh
nvsn init bash             # 🖨️ shows the block
nvsn init bash --apply     # ✍️ writes it to ~/.bashrc (after confirmation)
```

Details per shell in [Shell Integration](#-shell-integration).

---

### 🚫 `nvsn deactivate [--shell <shell>]`

Prints the shell code that removes the active version from the session. The integration calls it for you through the `nvsn` wrapper function (`nvsn deactivate`). Exits with 0 even when no version is active.

```sh
nvsn deactivate --shell bash
```

---

### 🏁 `nvsn completions <shell>`

Prints a shell completion script to stdout. Supported shells: `bash`, `zsh`, `fish`, `powershell`.

```sh
# 🐧 Bash
nvsn completions bash > ~/.local/share/bash-completion/completions/nvsn

# 🐚 Zsh
nvsn completions zsh > "${fpath[1]}/_nvsn"

# 🐟 Fish
nvsn completions fish > ~/.config/fish/completions/nvsn.fish

# 🪟 PowerShell
nvsn completions powershell >> $PROFILE
```

---

### 🩺 `nvsn doctor`

Checks the installation and reports problems with actionable hints:

- 📁 The data directory exists and is writable
- 🖥️ The platform (OS and architecture)
- 🐚 The detected shell and whether the integration is loaded
- 📦 Installed versions and the default version
- 🗂️ The cached index
- 🔍 Whether `bin` is in your `PATH`, and whether the profile hook is present

```sh
nvsn doctor
```

> Exits with code `1` if any check fails - useful for CI health checks.

---

### 🧹 `nvsn prune [--dry-run] [--force] [--scan-dir <PATH>]`

Removes installed versions that **nothing references**, freeing disk space without tracking down stale versions by hand. A version counts as referenced when it is the default, or when it appears in a `.nvmrc` or `.node-version` file found by walking up from the current directory (or while scanning `--scan-dir`, up to 5 levels deep). The version active in your session is never removed.

| Flag | Description |
| ---- | ----------- |
| `--dry-run` | Only list the versions that would be removed. |
| `--force` | Remove without asking for confirmation. |
| `--scan-dir <PATH>` | Also look for `.nvmrc` and `.node-version` files under this directory (5 levels). |

```sh
nvsn prune --dry-run                    # 👀 preview only
nvsn prune                              # 🔍 asks for confirmation
nvsn prune --force --scan-dir ~/projects
```

> Without a terminal and without `--force`, nothing is deleted and the command exits with code 9.

---

### 🔄 `nvsn self-update` (alias `upgrade`)

Updates nvsn to the latest release published on GitHub. The SHA-256 of the release is always verified; there is no option to skip it.

| Flag | Description |
| ---- | ----------- |
| `--check` | Only report whether a newer release exists. |
| `--force` | Reinstall the latest release even when you are up to date. |
| `--retries <N>` | Network retries with exponential back-off (1, 2, 4 s ...). Default `3`. |

```sh
nvsn self-update --check
nvsn self-update
```

> 🔔 nvsn also checks for a newer release in the background after most commands (cached for 24 hours, never adds noticeable delay) and prints a short notice when one exists. It is disabled by `NVSN_NO_UPDATE_CHECK=1`, when `CI` is set, or with `--offline`. It never runs for `env`, `path`, `deactivate`, `completions`, the internal hook, or `self-update` itself.

---

### 🧼 `nvsn self-uninstall`

Removes the nvsn binary, its data directory (`NVSN_DIR`) and the nvsn blocks in your shell profiles. It asks for confirmation unless `--yes` is given. It never touches anything else in your profiles.

```sh
nvsn self-uninstall
```

---

### 💣 `nvsn implode`

**Completely removes nvsn** and everything it manages: the data directory with all installed Node.js versions, the binary and the shell profile blocks.

| Flag | Description |
| ---- | ----------- |
| `--force` | Remove everything without asking. |

```sh
nvsn implode           # 🗑️ shows a summary, asks for confirmation
nvsn implode --force   # 💥 no questions asked
```

> ⚠️ This operation is **irreversible**: your installed Node.js versions are deleted. To update instead, use `nvsn self-update`.

---

## 🔢 Version Syntax

Every command that takes a version accepts these forms:

| Input | Meaning |
| ----- | ------- |
| `latest`, `node` | 🆕 Newest stable release |
| `lts/*`, `lts` | 🛟 Newest LTS line |
| `lts/<codename>` | 🏷️ LTS line by codename, case-insensitive: `lts/iron` |
| `lts/-N` | ⏪ The Nth LTS counting from the newest: `lts/-1` is the one before it |
| `24` | 🔢 Highest release in the 24 line |
| `24.21` | 🔢 Highest patch of 24.21 |
| `24.21.0` or `v24.21.0` | 🎯 Exact version (the `v` is optional) |
| `^20`, `~20.11`, `20.x`, `>=18 <21` | 📐 Semver ranges |

Not supported: `stable` and `unstable` (obsolete in nvm; they return an error with exit code 2), `iojs` and `io.js`, and `system` / `current` in remote commands. User aliases work in `use`, `run`, `exec`, `which`, `uninstall`, `default` and `version`.

> 💡 A partial specification or a range never picks a prerelease when no stable release matches.

---

## 📌 Per-project Versions

Place a `.nvmrc` (or a `.node-version`) file in your project:

```
20.11.1
```

- nvsn searches from the current directory upwards and uses the **first** file it finds. Inside one directory, `.nvmrc` wins over `.node-version`.
- A file holds **exactly one** version. Comments starting with `#`, surrounding spaces, a UTF-8 byte-order mark and CRLF line endings are accepted. Two or more version lines are an error (exit code 3).
- With the shell integration loaded, entering the directory activates its version. If that version is not installed, the hook does nothing: install it with `nvsn install`.
- `nvsn local <version>` writes the file for you.

---

## 🐚 Shell Integration

`nvsn init <shell> --apply` writes a small block into your profile. The block evaluates the output of `nvsn env` when a shell starts, and defines an `nvsn` function that applies the output of `nvsn use` to the current session.

| Shell | Profile file | Block |
| ----- | ------------ | ----- |
| 🐧 Bash | `~/.bashrc` | `eval "$(nvsn env --shell bash)"` |
| 🐚 Zsh | `~/.zshrc` | `eval "$(nvsn env --shell zsh)"` |
| 🐟 Fish | `~/.config/fish/config.fish` | `nvsn env --shell fish \| source` |
| 🪟 PowerShell 7 | `Documents\PowerShell\Microsoft.PowerShell_profile.ps1` | `nvsn env --shell powershell \| Out-String \| Invoke-Expression` |
| 🪟 Windows PowerShell 5.1 | `Documents\WindowsPowerShell\Microsoft.PowerShell_profile.ps1` | Same block as PowerShell 7 (`init powershell --apply` writes both) |

On every directory change the hook:

1. 🔍 Checks, inside the shell, whether the current directory or a parent has a `.nvmrc` or `.node-version`. If not, the binary is not started at all.
2. ➕ If there is one, runs the internal `nvsn __hook` and applies the version it selects.
3. 🔙 When you leave a directory whose version was activated automatically and the new directory has no version file, it deactivates that automatic version.

A manual `nvsn use` takes priority over the hook for the rest of the session.

**Login profile and Windows PATH** - so that graphical applications (editors and similar) find the default version without a shell:

| Platform | Where | What |
| -------- | ----- | ---- |
| 🐧 Linux (bash) | `~/.profile` | A `# nvsn path` block that adds the `current` link to `PATH` |
| 🐚 Linux and macOS (zsh) | `~/.zprofile` | The same `# nvsn path` block |
| 🪟 Windows | `HKCU\Environment` | `init --apply` adds the nvsn binary folder and `<NVSN_DIR>\current` to your user PATH |

> 🔇 nvsn runs no daemons and no background processes, and it never changes the system PATH.

---

## ⚙️ Configuration

nvsn is configured with command-line flags and environment variables. Flags take precedence over environment variables, which take precedence over the defaults. There is no configuration file yet.

| Variable | Default | Description |
| -------- | ------- | ----------- |
| `NVSN_DIR` | Per platform (see below) | 📁 Root directory for installed versions, cache, aliases and the default |
| `NVSN_NODEJS_ORG_MIRROR` | `https://nodejs.org/dist` | 🪞 Node.js distribution mirror. HTTPS only, unless `--allow-http` |
| `NVSN_NO_UPDATE_CHECK` | unset | 🔕 Set to `1` to disable the background update notice |
| `NVSN_REPO` | `jhonsferg/nvsn` | 🔄 Repository used by `self-update` (`owner/name`) |
| `NVSN_INSTALL_DIR` | `~/.local/bin` (`%USERPROFILE%\.local\bin`) | 📂 Installer only: directory for the binary |
| `NVSN_VERSION` | `latest` | 🏷️ Installer only: release tag to install (for example `v0.1.0`). The binary uses the same name for the active session version |
| `NO_COLOR` | unset | 🎨 Disables colors |
| `CI` | unset | 🤖 When set, the background update notice is skipped |

### 📁 Default data directory

- **Windows**: `%LOCALAPPDATA%\nvsn`
- **Linux and macOS**: `$XDG_DATA_HOME/nvsn` if `XDG_DATA_HOME` is an absolute path, otherwise `~/.local/share/nvsn`
- **Android (Termux)**: `~/.nvsn`

### 📂 Directory layout

```
<NVSN_DIR>/
|-- default                         # default version (plain text)
|-- current -> versions/v24.21.0/   # link to the default version (junction on Windows)
|-- versions/
|   |-- v24.21.0/                   # extracted Node.js distribution
|   `-- ...
|-- aliases/                        # one file per alias
|-- cache/
|   |-- downloads/                  # partial downloads, resumable
|   `-- index/                      # cached nodejs.org index
|-- locks/                          # one lock file per version being installed
`-- lock                            # store lock (aliases and default)
```

---

## 🛠️ Building from Source

The workspace pins its Rust toolchain in `rust-toolchain.toml` (1.98.0); `rustup` installs it automatically. No system dependencies are needed: TLS is handled by [rustls](https://github.com/rustls/rustls), which is pure Rust and does not need OpenSSL.

```sh
git clone https://github.com/jhonsferg/nvsn.git
cd nvsn
cargo build --release -p nvsn-cli
```

The binary is placed at `target/release/nvsn` (or `nvsn.exe` on Windows). Copy it to a directory in your `PATH`.

```sh
# ✅ Run the self-check after building
./target/release/nvsn doctor
```

---

## 🧪 Running the Test Suite

```sh
cargo test --workspace      # unit and integration tests
cargo fmt --all -- --check  # formatting
cargo clippy --workspace --all-targets -- -D warnings  # lints
cargo deny check            # advisories, license allowlist, source checks
```

The integration tests spawn real child processes and a local mock server for network-dependent commands, so they need no internet access and nothing outside a temporary directory.

---

## 📊 Benchmarks

nvsn ships [criterion](https://github.com/bheisler/criterion.rs) benchmarks for its hottest pure-computation paths (version-spec parsing, index resolution, the `.nvmrc` upward search, checksum verification, index parsing, and shell-profile parsing). They live in each crate's `.benches/` directory and are picked from real call-graph fan-in, not guessed; see each benchmark file's module doc comment for the rationale.

```sh
cargo bench -p nvsn-core
cargo bench -p nvsn-shell
```

HTML reports land in `target/criterion/report/index.html`.

These are local-only: they run for several seconds per input to get a statistically stable measurement, which would be wasted CI compute on every push, so no GitHub workflow in this repository invokes `cargo bench`. `cargo test --workspace --all-targets` does compile and run the benchmark binaries once each, in criterion's fast smoke-test mode (no statistical sampling), just to catch a panic or a broken fixture.

Benchmarks measure CPU time only, not memory or file descriptor/handle leaks. To check for those, run an install/uninstall loop while sampling the process:

```sh
# Linux: peak RSS and open file descriptors
/usr/bin/time -v ./target/release/nvsn install 20 --no-use
ls /proc/<pid>/fd | wc -l   # across repeated cycles; a growing count points at a leak

# Windows (PowerShell): working-set memory and handle count
Get-Process -Id <pid> | Select-Object WS, PM, HandleCount
```

nvsn's processes are short-lived by design (one operation, then exit), so there is no long-running daemon whose memory would grow unbounded; the main risk to watch for is disk (leftover `.tmp-*` staging directories, locks never released) and handles held open across the lifetime of a single command, not heap growth over hours of uptime.

---

## 📦 Release Artifacts

The release workflow (`.github/workflows/release.yml`) cross-compiles these archives when a version tag is pushed:

| Artifact | Target | Notes |
| -------- | ------ | ----- |
| `nvsn_windows_x86_64.zip` | `x86_64-pc-windows-msvc` | ⚡ Static CRT |
| `nvsn_windows_arm64.zip` | `aarch64-pc-windows-msvc` | ⚡ Static CRT |
| `nvsn_linux_x86_64.tar.gz` | `x86_64-unknown-linux-musl` | ⚡ Static binary |
| `nvsn_linux_aarch64.tar.gz` | `aarch64-unknown-linux-musl` | ⚡ Static binary |
| `nvsn_linux_armv7.tar.gz` | `armv7-unknown-linux-musleabihf` | ⚡ Static binary |
| `nvsn_linux_386.tar.gz` | `i686-unknown-linux-musl` | ⚡ Static binary |
| `nvsn_linux_riscv64.tar.gz` | `riscv64gc-unknown-linux-gnu` | Links the system glibc |
| `nvsn_linux_s390x.tar.gz` | `s390x-unknown-linux-gnu` | IBM Z, links the system glibc |
| `nvsn_linux_ppc64le.tar.gz` | `powerpc64le-unknown-linux-gnu` | IBM POWER LE, links the system glibc |
| `nvsn_android_aarch64.tar.gz` | `aarch64-linux-android` | 🤖 Termux, links bionic (not verified) |
| `nvsn_darwin_x86_64.tar.gz` | `x86_64-apple-darwin` | Links libSystem (normal on macOS) |
| `nvsn_darwin_aarch64.tar.gz` | `aarch64-apple-darwin` | 🍎 Apple Silicon, links libSystem |

Each release also includes `checksums.txt` with SHA-256 hashes for all artifacts, plus SBOM files in CycloneDX and SPDX formats.

Releases are automated: the `auto-release` job in `ci.yml` creates a version tag on push to `main` (the version bump follows the Conventional Commit prefixes) and dispatches `release.yml`.

---

## 🔒 Verifying you're on the real project

Only trust downloads that come from **GitHub Releases on this exact repository**:
`https://github.com/jhonsferg/nvsn/releases`

Do not trust:
- Download buttons or "release" links that point to a raw file inside a repository tree instead of the Releases page.
- Copies of this project hosted under a different GitHub account, even if the name, README and commit history look identical. Those copies are not maintained by this project.
- Instructions to bypass a Windows SmartScreen, macOS Gatekeeper or antivirus warning for an `nvsn` binary. Genuine releases come with `checksums.txt` and do not require disabling operating-system protections.

If you find a copy of this project distributing something other than the source in this repository, especially compiled binaries that were not produced by the release workflow, please report it privately - see [SECURITY.md](SECURITY.md#malicious-forks--clones).

---

## 🧪 Status and limitations

nvsn is in **alpha**. It works for the daily use described in this README, but the test coverage on real platforms is limited.

**Tested:**

- 🪟 **Windows 11 x64**: CLI commands run against the real Node.js index (install, `use`, `run`, `exec`, `default`, `alias`, `doctor`, `init`).
- 🐧 **Linux**: the `install.sh` installer was tested in a Linux VM and in a clean container.
- 🐚 **Shells**: bash 5.3 (Git Bash), dash, PowerShell 7 and Windows PowerShell 5.1 with real tests; zsh 5.9 and fish 4.0.2 in a Debian container.

**Not verified yet:**

- Windows ARM64, macOS (x86_64 and ARM64) and Linux on ARM64, ARMv7 and other architectures. They are listed in the release matrix, but have not been built or tested locally.
- The GitHub Actions workflows in `.github/workflows/` have been written but never run on GitHub.

**Not available:**

- 📱 **Android (Termux)** - the release matrix builds `aarch64-linux-android` with `cross`, but the binary has not been run on a device. nvsn does not yet install Node.js through Termux's package manager, so `nvsn install` refuses on Termux with exit code 10 (`unsupported`).
- 🏗️ **Building Node.js from source** - there is no `nvsn build` and no `install -s`.
- 🔐 **Signature verification** - only the SHA-256 checksum is verified; the optional GPG signature check is not implemented.
- 🔁 **Importing from nvm** (`import-nvm`) and shims for IDEs and cron jobs - not implemented.
- 🐚 **Other shells** - nushell, elvish, xonsh, cmd.exe and sh/dash/ash have no integration, so the version does not switch automatically when you enter a directory. Use `nvsn run` and `nvsn exec` there.
- 🚫 **io.js, `stable` and `unstable`** - not supported, by design.

---

## 📄 License

Licensed under the [MIT License](LICENSE).

Parts of this software are derived from gvsn 1.5.1, a Go version manager by the same author, published under the MIT license.

---

<div align="center">

Made with 🦀 Rust · Maintained with ❤️

</div>
