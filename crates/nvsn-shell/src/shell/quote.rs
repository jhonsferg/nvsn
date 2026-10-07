//! Escaping of values (paths, versions) per shell dialect.
//!
//! Each function returns a word ready to paste into the generated script.
//! Paths can contain spaces, accents, quotes and characters that each
//! shell interprets (`$`, `%`, `\`, typographic quotes in PowerShell). No
//! function depends on the platform: the output text is the same on Linux,
//! macOS and Windows.

/// POSIX single quotes (sh, bash, zsh): each `'` is written as `'\''`.
pub fn posix(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\\''"))
}

/// Escapes `raw` for use inside POSIX double quotes: `\`, `"`, `$` and
/// backtick are prefixed with `\`. It does not add the outer quotes.
pub fn posix_dq_body(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        if matches!(c, '\\' | '"' | '$' | '`') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// fish single quotes: `\` and `'` are escaped with `\`.
pub fn fish(raw: &str) -> String {
    let escaped = raw.replace('\\', "\\\\").replace('\'', "\\'");
    format!("'{escaped}'")
}

/// Quotes that PowerShell treats as a single quote (ASCII and typographic).
const PS_SINGLE_QUOTES: [char; 5] = ['\'', '\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'];

/// PowerShell 5.1 and 7+ single quotes. Each single quote, including the
/// typographic ones that PowerShell also recognizes, is doubled.
pub fn powershell(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('\'');
    for c in raw.chars() {
        if PS_SINGLE_QUOTES.contains(&c) {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test path with a space, accents, a single quote, a double quote and `$`.
    const RAW: &str = r#"C:\Users\José Ñu\it's "x" $HOME"#;

    #[test]
    fn posix_escapes_single_quote_and_keeps_backslash() {
        assert_eq!(posix(RAW), r#"'C:\Users\José Ñu\it'\''s "x" $HOME'"#);
    }

    #[test]
    fn posix_dq_body_escapes_dollar_and_backslash() {
        assert_eq!(posix_dq_body("/home/jo/Mi Dir/$x"), "/home/jo/Mi Dir/\\$x");
        assert_eq!(posix_dq_body(r#"a\"b`c"#), r#"a\\\"b\`c"#);
    }

    #[test]
    fn fish_escapes_backslash_and_single_quote() {
        assert_eq!(fish(RAW), r#"'C:\\Users\\José Ñu\\it\'s "x" $HOME'"#);
    }

    #[test]
    fn powershell_doubles_ascii_and_typographic_quotes() {
        assert_eq!(powershell(RAW), r#"'C:\Users\José Ñu\it''s "x" $HOME'"#);
        assert_eq!(powershell("a\u{2019}b"), "'a\u{2019}\u{2019}b'");
    }
}
