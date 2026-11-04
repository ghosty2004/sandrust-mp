//! The connection arguments the launcher passes to gta_sa.exe, using the same
//! convention as the original SA:MP launcher: `-c -n NAME -h HOST -p PORT`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectArgs {
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    /// `-c` was present.
    pub connect: bool,
}

/// Parses the arguments after the executable name. Unknown flags are ignored.
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> ConnectArgs {
    let mut out = ConnectArgs { name: None, host: None, port: None, connect: false };
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-c" => out.connect = true,
            "-n" => out.name = it.next(),
            "-h" => out.host = it.next(),
            "-p" => out.port = it.next().and_then(|p| p.parse().ok()),
            _ => {}
        }
    }
    out
}

/// Builds the argument list for gta_sa.exe (without the executable name).
pub fn to_args(name: &str, host: &str, port: u16) -> Vec<String> {
    ["-c", "-n", name, "-h", host, "-p", &port.to_string()].iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let a = parse(to_args("Rusty", "127.0.0.1", 7777));
        assert_eq!(
            a,
            ConnectArgs { name: Some("Rusty".into()), host: Some("127.0.0.1".into()), port: Some(7777), connect: true }
        );
    }

    #[test]
    fn tolerates_garbage() {
        let a = parse(["-x".to_string(), "-p".into(), "notaport".into(), "-n".into()]);
        assert_eq!(a, ConnectArgs { name: None, host: None, port: None, connect: false });
    }
}
