//! Hand-rolled CLI parsing for `minikvm run <guest.bin> [--trace] [--gdb [--gdb-port N]]`.
//! No clap yet (YAGNI) — one subcommand, one positional, a few flags.

#[derive(Debug, PartialEq, Eq)]
pub struct Config {
    pub guest_path: String,
    pub trace: bool,
    pub gdb: bool,
    pub gdb_port: u16,
}

/// Parse args *after* the program name (i.e. `std::env::args().skip(1)`).
pub fn parse_args(args: &[String]) -> Result<Config, String> {
    let mut iter = args.iter();
    match iter.next().map(String::as_str) {
        Some("run") => {}
        Some(other) => return Err(format!("unknown command '{other}'; expected 'run'")),
        None => {
            return Err(
                "usage: minikvm run <guest.bin> [--trace] [--gdb [--gdb-port N]]".to_string(),
            );
        }
    }

    let mut guest_path: Option<String> = None;
    let mut trace = false;
    let mut gdb = false;
    let mut gdb_port: u16 = 1234;
    let mut gdb_port_explicit = false;

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--trace" => trace = true,
            "--gdb" => gdb = true,
            s if s.starts_with("--gdb-port=") => {
                return Err(
                    "--gdb-port takes a separate argument; use --gdb-port N".to_string()
                );
            }
            "--gdb-port" => {
                let n = iter
                    .next()
                    .ok_or_else(|| "--gdb-port requires a value".to_string())?;
                gdb_port = n
                    .parse::<u16>()
                    .map_err(|_| format!("--gdb-port: '{n}' is not a valid u16"))?;
                gdb_port_explicit = true;
            }
            s if s.starts_with('-') => return Err(format!("unknown flag '{s}'")),
            s => {
                if guest_path.is_some() {
                    return Err(format!("unexpected extra argument '{s}'"));
                }
                guest_path = Some(s.to_string());
            }
        }
    }

    if gdb_port_explicit && !gdb {
        return Err("--gdb-port requires --gdb".to_string());
    }

    let guest_path = guest_path.ok_or("missing guest image path")?;
    Ok(Config { guest_path, trace, gdb, gdb_port })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_run_with_trace() {
        let cfg = parse_args(&v(&["run", "guest/hello.bin", "--trace"])).unwrap();
        assert_eq!(
            cfg,
            Config {
                guest_path: "guest/hello.bin".to_string(),
                trace: true,
                gdb: false,
                gdb_port: 1234,
            }
        );
    }

    #[test]
    fn trace_defaults_off() {
        let cfg = parse_args(&v(&["run", "g.bin"])).unwrap();
        assert!(!cfg.trace);
    }

    #[test]
    fn missing_path_is_error() {
        assert!(parse_args(&v(&["run"])).is_err());
    }

    #[test]
    fn unknown_command_is_error() {
        assert!(parse_args(&v(&["boot", "g.bin"])).is_err());
    }

    #[test]
    fn gdb_flag_sets_field() {
        let c = parse_args(&v(&["run", "guest.img", "--gdb"])).unwrap();
        assert!(c.gdb);
        assert_eq!(c.gdb_port, 1234);
    }

    #[test]
    fn gdb_port_explicit() {
        let c = parse_args(&v(&["run", "guest.img", "--gdb", "--gdb-port", "5555"])).unwrap();
        assert!(c.gdb);
        assert_eq!(c.gdb_port, 5555);
    }

    #[test]
    fn gdb_port_zero_is_allowed() {
        let c = parse_args(&v(&["run", "guest.img", "--gdb", "--gdb-port", "0"])).unwrap();
        assert_eq!(c.gdb_port, 0);
    }

    #[test]
    fn gdb_port_without_gdb_rejected() {
        let err = parse_args(&v(&["run", "guest.img", "--gdb-port", "1234"])).unwrap_err();
        assert!(err.contains("--gdb-port requires --gdb"), "got: {err}");
    }

    #[test]
    fn gdb_port_equals_form_rejected() {
        let err = parse_args(&v(&["run", "guest.img", "--gdb", "--gdb-port=5555"])).unwrap_err();
        assert!(err.contains("takes a separate argument"), "got: {err}");
    }

    #[test]
    fn no_gdb_flags_keeps_defaults() {
        let c = parse_args(&v(&["run", "guest.img"])).unwrap();
        assert!(!c.gdb);
        assert_eq!(c.gdb_port, 1234);
    }
}
