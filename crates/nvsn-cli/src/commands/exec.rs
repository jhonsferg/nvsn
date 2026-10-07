//! `exec`: runs a command with the binary directory of a version at the
//! start of the child's `PATH`. It does not change the session or the parent's `PATH`.

use crate::commands::run::{finish, strip_separator};
use crate::ctx::Ctx;
use crate::exit::CliError;
use crate::versions::{bin_dir, resolve_installed};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Runs `command` with `args`, with the version directory first in `PATH`.
///
/// The directory is `bin/` when it exists (Unix) and otherwise the root of the version
/// (Windows, where `node.exe` and `npm.cmd` are in the root). It is the same one that
/// `nvsn use` uses.
///
/// On Windows the command is looked up as with `PATHEXT`: `npm` resolves to `npm.cmd`.
/// `.cmd` and `.bat` files are run with `cmd /C`; see `cmd_command_line` for its
/// escaping limitations.
///
/// # Errors
///
/// `version-not-found` (6) if the version is not installed, `usage` (2) if the
/// specification is not valid or an argument cannot be passed to a `.cmd`/`.bat`,
/// general error (1) if the command does not exist in the version's `PATH` or
/// cannot be launched. If the child exits with an error, it exits with the same code.
pub fn exec_command(
    ctx: &Ctx,
    version: &str,
    command: &str,
    args: &[String],
) -> Result<(), CliError> {
    let inst = resolve_installed(ctx, version)?;
    let path = path_with_first(&bin_dir(&inst.dir))?;

    let mut child = child_command(command, strip_separator(args), &path)?;
    let status = child
        .env("PATH", &path)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|err| spawn_error(command, &inst.tag, &err))?;
    finish(status);
    Ok(())
}

/// Returns the current `PATH` with `dir` in front.
fn path_with_first(dir: &Path) -> Result<OsString, CliError> {
    let mut paths: Vec<PathBuf> = vec![dir.to_path_buf()];
    if let Some(current) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&current));
    }
    std::env::join_paths(paths)
        .map_err(|err| CliError::general(format!("cannot build PATH: {err}")))
}

/// Translates the launch error. If the command does not exist, the message says so.
fn spawn_error(command: &str, tag: &str, err: &std::io::Error) -> CliError {
    if err.kind() == std::io::ErrorKind::NotFound {
        CliError::general(format!(
            "command '{command}' not found in the PATH of {tag}"
        ))
        .with_hint("check the command name; the version directory is searched first")
    } else {
        CliError::general(format!("cannot run '{command}': {err}"))
    }
}

/// Unix command: the name as is, resolved through the child's `PATH`.
#[cfg(not(windows))]
fn child_command(command: &str, args: &[String], _path: &OsStr) -> Result<Command, CliError> {
    let mut cmd = Command::new(command);
    cmd.args(args);
    Ok(cmd)
}

/// Windows command: the executable resolved with `PATHEXT`. `.cmd` and `.bat`
/// are not Win32 executables, so they are launched with `cmd /d /s /c`.
#[cfg(windows)]
fn child_command(command: &str, args: &[String], path: &OsStr) -> Result<Command, CliError> {
    use std::os::windows::process::CommandExt;

    let program = resolve_windows(command, path);
    if !is_batch_script(&program) {
        let mut cmd = Command::new(&program);
        cmd.args(args);
        return Ok(cmd);
    }

    let line = cmd_command_line(&program, args)?;
    let mut cmd = Command::new(cmd_exe());
    cmd.raw_arg(format!("/d /s /c \"{line}\""));
    Ok(cmd)
}

/// Looks up `command` in the child's `PATH` with the extensions from `PATHEXT`.
#[cfg(windows)]
fn resolve_windows(command: &str, path: &OsStr) -> PathBuf {
    let dirs: Vec<PathBuf> = std::env::split_paths(path).collect();
    find_in_dirs(command, &dirs, &pathext()).unwrap_or_else(|| PathBuf::from(command))
}

/// Extensions from `PATHEXT`, or the Windows defaults if it is not defined.
#[cfg(windows)]
fn pathext() -> Vec<String> {
    let raw = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
    raw.split(';')
        .map(str::trim)
        .filter(|ext| !ext.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Command interpreter: `COMSPEC` if it exists, or `cmd.exe` from the `PATH`.
#[cfg(windows)]
fn cmd_exe() -> PathBuf {
    std::env::var_os("COMSPEC")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("cmd.exe"))
}

/// Command line for `cmd /C`: the script path and the arguments. Empty arguments or
/// arguments with spaces and cmd metacharacters are quoted; the rest is passed as is,
/// so that `%1` in the script receives the value without quotes.
///
/// Escaping limitations (cmd.exe has no reliable escaping for quotes):
/// - An argument with `"` or a line break is rejected with `usage` (2).
/// - Inside quotes, `&`, `|`, `<`, `>`, `(`, `)` and `^` are literal, but
///   `%NAME%` is expanded if the variable exists. There is no escape for `%`.
/// - `.cmd` and `.bat` files receive the line as is: the script decides how to parse
///   its arguments (`%1`, `%*`).
#[cfg(windows)]
fn cmd_command_line(program: &Path, args: &[String]) -> Result<String, CliError> {
    const SPECIAL: &str = "&|<>^()%!,;=";
    let mut line = format!("\"{}\"", program.display());
    for arg in args {
        if arg.contains(['"', '\r', '\n']) {
            return Err(CliError::usage(format!(
                "argument '{arg}' cannot be passed to a .cmd or .bat command"
            ))
            .with_hint("remove the double quotes or line breaks from the argument"));
        }
        let needs_quotes =
            arg.is_empty() || arg.contains(|c: char| c.is_whitespace() || SPECIAL.contains(c));
        line.push(' ');
        if needs_quotes {
            line.push('"');
            line.push_str(arg);
            line.push('"');
        } else {
            line.push_str(arg);
        }
    }
    Ok(line)
}

/// Looks up `command` in `dirs`, in order. For each directory it tries the name as is
/// (only if it already has an extension from `exts`, because the Node distribution
/// for Windows ships an `npm` script without extension that must not shadow `npm.cmd`),
/// and then the name with each extension from `exts`. If `command` contains a
/// path separator, it is not searched in `dirs`.
#[cfg(any(windows, test))]
fn find_in_dirs(command: &str, dirs: &[PathBuf], exts: &[String]) -> Option<PathBuf> {
    let bases: Vec<PathBuf> = if command.contains(['/', '\\']) {
        vec![PathBuf::from(command)]
    } else {
        dirs.iter().map(|dir| dir.join(command)).collect()
    };

    for base in bases {
        if has_known_ext(&base, exts) && base.is_file() {
            return Some(base);
        }
        for ext in exts {
            let mut name = base.clone().into_os_string();
            name.push(ext);
            let candidate = PathBuf::from(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// `true` if `path` already ends in one of `exts` (case insensitive).
#[cfg(any(windows, test))]
fn has_known_ext(path: &Path, exts: &[String]) -> bool {
    path.extension().and_then(OsStr::to_str).is_some_and(|ext| {
        exts.iter()
            .any(|known| known.trim_start_matches('.').eq_ignore_ascii_case(ext))
    })
}

/// `true` if the executable is a `.cmd` or `.bat` script.
#[cfg(windows)]
fn is_batch_script(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_directory_goes_first() {
        let path = path_with_first(Path::new("/opt/nvsn/v20/bin")).expect("path");
        let parts: Vec<PathBuf> = std::env::split_paths(&path).collect();
        assert_eq!(
            parts.first().map(PathBuf::as_path),
            Some(Path::new("/opt/nvsn/v20/bin"))
        );
    }

    #[test]
    fn missing_command_is_a_general_error_with_hint() {
        let err = std::io::Error::from(std::io::ErrorKind::NotFound);
        let cli = spawn_error("nope", "v20.11.1", &err);
        assert_eq!(cli.exit_code(), crate::exit::GENERAL);
        assert!(cli.message.contains("'nope' not found"));
        assert!(cli.hint.is_some());
    }

    fn exts() -> Vec<String> {
        [".COM", ".EXE", ".BAT", ".CMD"]
            .iter()
            .map(|ext| (*ext).to_owned())
            .collect()
    }

    fn touch(path: &Path) {
        std::fs::write(path, b"x").expect("write file");
    }

    #[test]
    fn bare_name_resolves_to_the_pathext_script() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(&dir.path().join("npm"));
        touch(&dir.path().join("npm.CMD"));
        let found = find_in_dirs("npm", &[dir.path().to_path_buf()], &exts());
        assert_eq!(found, Some(dir.path().join("npm.CMD")));
    }

    #[test]
    fn name_with_a_known_extension_is_tried_as_is() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(&dir.path().join("node.exe"));
        let found = find_in_dirs("node.exe", &[dir.path().to_path_buf()], &exts());
        assert_eq!(found, Some(dir.path().join("node.exe")));
    }

    #[test]
    fn earlier_directory_wins_over_pathext_order() {
        let first = tempfile::tempdir().expect("tempdir");
        let second = tempfile::tempdir().expect("tempdir");
        touch(&first.path().join("tool.BAT"));
        touch(&second.path().join("tool.CMD"));
        let dirs = [first.path().to_path_buf(), second.path().to_path_buf()];
        assert_eq!(
            find_in_dirs("tool", &dirs, &exts()),
            Some(first.path().join("tool.BAT"))
        );
    }

    #[test]
    fn missing_command_resolves_to_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            find_in_dirs("nvsn-no-such-tool", &[dir.path().to_path_buf()], &exts()),
            None
        );
    }

    #[test]
    fn command_with_a_separator_ignores_the_path_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let script = dir.path().join("run.CMD");
        touch(&script);
        let command = script.with_extension("");
        let found = find_in_dirs(
            command.to_str().expect("utf-8 temp path"),
            &[PathBuf::from("/nonexistent")],
            &exts(),
        );
        assert_eq!(found, Some(script));
    }

    #[cfg(windows)]
    #[test]
    fn cmd_command_line_rejects_double_quotes() {
        let err = cmd_command_line(Path::new(r"C:\x\npm.cmd"), &["a\"b".to_owned()])
            .expect_err("quotes must be rejected");
        assert_eq!(err.exit_code(), crate::exit::USAGE);
    }

    #[cfg(windows)]
    #[test]
    fn cmd_command_line_quotes_only_arguments_that_need_it() {
        let line = cmd_command_line(
            Path::new(r"C:\x\npm.cmd"),
            &["install".to_owned(), "a&b c".to_owned(), String::new()],
        )
        .expect("line");
        assert_eq!(line, r#""C:\x\npm.cmd" install "a&b c" """#);
    }
}
