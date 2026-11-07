//! The connection arguments the launcher passes to gta_sa.exe, using the same
//! convention as the original SA:MP launcher: `-c -n NAME -h HOST -p PORT [-z PASSWORD]`.
//! Spec: docs/re/handshake.md, "Command line".

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectArgs {
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    /// `-z`: the server password; `None` when absent or empty (both send no password).
    pub password: Option<String>,
    /// `-c` was present.
    pub connect: bool,
}

/// samp reads a switch's value as one word: it stops at the first space, `-` or `/`, so a value never
/// contains them. Mirrors samp.dll 0x100C3F00.
fn word(value: &str) -> String {
    value.trim_start_matches(' ').split([' ', '-', '/']).next().unwrap_or("").to_string()
}

/// Parses the arguments after the executable name. Unknown flags are ignored.
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> ConnectArgs {
    let mut out = ConnectArgs { name: None, host: None, port: None, password: None, connect: false };
    let mut it = args.into_iter().peekable();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-c" => out.connect = true,
            "-n" => out.name = it.next(),
            "-h" => out.host = it.next(),
            "-p" => out.port = it.next().and_then(|p| p.parse().ok()),
            // A following switch is not a value: samp's word ends at its `-`, the password is empty and
            // the switch still counts.
            "-z" => {
                let value = it.next_if(|v| !v.starts_with(['-', '/'])).map(|v| word(&v)).unwrap_or_default();
                out.password = (!value.is_empty()).then_some(value);
            }
            _ => {}
        }
    }
    out
}

/// Builds the argument list for gta_sa.exe (without the executable name); `-z` only with a password.
pub fn to_args(name: &str, host: &str, port: u16, password: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> =
        ["-c", "-n", name, "-h", host, "-p", &port.to_string()].iter().map(|s| s.to_string()).collect();
    if let Some(p) = password {
        args.extend(["-z".to_string(), p.to_string()]);
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn roundtrip() {
        let a = parse(to_args("Rusty", "127.0.0.1", 7777, None));
        assert_eq!(
            a,
            ConnectArgs {
                name: Some("Rusty".into()),
                host: Some("127.0.0.1".into()),
                port: Some(7777),
                password: None,
                connect: true
            }
        );
        let a = parse(to_args("Rusty", "127.0.0.1", 7777, Some("s3cret")));
        assert_eq!(a.password.as_deref(), Some("s3cret"));
        assert_eq!(a.name.as_deref(), Some("Rusty"));
    }

    #[test]
    fn password_switch_reads_one_word() {
        assert_eq!(parse(s(&["-z", "abc"])).password.as_deref(), Some("abc"));
        // The word ends at `-` or `/`.
        assert_eq!(parse(s(&["-z", "ab-cd"])).password.as_deref(), Some("ab"));
        assert_eq!(parse(s(&["-z", "ab/cd"])).password.as_deref(), Some("ab"));
        // Empty or missing value: no password, and a following switch still applies.
        assert_eq!(parse(s(&["-z", ""])).password, None);
        assert_eq!(parse(s(&["-z"])).password, None);
        let a = parse(s(&["-z", "-n", "Rusty"]));
        assert_eq!((a.password, a.name.as_deref()), (None, Some("Rusty")));
    }

    #[test]
    fn tolerates_garbage() {
        let a = parse(s(&["-x", "-p", "notaport", "-n"]));
        assert_eq!(a, ConnectArgs { name: None, host: None, port: None, password: None, connect: false });
    }
}
