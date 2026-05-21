//! Hand-rolled CLI parsing for `minikvm run <guest.bin> [--trace]`.
//! No clap yet (YAGNI) — one subcommand, one positional, one flag.

#[derive(Debug, PartialEq, Eq)]
pub struct Config {
    pub guest_path: String,
    pub trace: bool,
}

/// Parse args *after* the program name (i.e. `std::env::args().skip(1)`).
pub fn parse_args(args: &[String]) -> Result<Config, String> {
    let mut iter = args.iter();
    match iter.next().map(String::as_str) {
        Some("run") => {}
        Some(other) => return Err(format!("unknown command '{other}'; expected 'run'")),
        None => return Err("usage: minikvm run <guest.bin> [--trace]".to_string()),
    }

    let mut guest_path: Option<String> = None;
    let mut trace = false;
    for arg in iter {
        match arg.as_str() {
            "--trace" => trace = true,
            s if s.starts_with('-') => return Err(format!("unknown flag '{s}'")),
            s => {
                if guest_path.is_some() {
                    return Err(format!("unexpected extra argument '{s}'"));
                }
                guest_path = Some(s.to_string());
            }
        }
    }

    let guest_path = guest_path.ok_or("missing guest image path")?;
    Ok(Config { guest_path, trace })
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
            Config { guest_path: "guest/hello.bin".to_string(), trace: true }
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
}
