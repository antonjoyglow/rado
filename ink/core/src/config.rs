//! The device's config file, as Rado's Settings → General → PocketBook → Copy config writes it.

/// Where the config lives on the PocketBook (its system config folder).
pub const CONFIG_PATH: &str = "/mnt/ext1/system/config/rado.cfg";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub token: String,
}

/// The config from the file's contents as read; the error names the file and what is wrong with it.
pub fn load(path: &str, read: std::io::Result<String>) -> Result<Config, String> {
    match read {
        Ok(text) => parse(&text).map_err(|e| format!("{path} can't be used. {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(format!("There is no {path}.")),
        Err(e) => Err(format!("{path} can't be read: {e}.")),
    }
}

/// Reads `host=`, `port=` and `token=` lines. The error says what is missing or wrong.
fn parse(text: &str) -> Result<Config, String> {
    let text = text.trim_start_matches('\u{feff}');
    let value = |key: &str| {
        text.lines()
            .filter_map(|line| line.split_once('='))
            .find(|(k, _)| k.trim() == key)
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or(format!("It has no {key}= line."))
    };
    let host = value("host")?;
    let port = value("port")?;
    let port = port.parse().map_err(|_| format!("Its port, {port}, is not a port number."))?;
    let token = value("token")?;
    Ok(Config { host, port, token })
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    #[test]
    fn parse_radosCopiedConfig() {
        let text = "\u{feff}host=192.168.1.14\r\nport=7878\r\ntoken=abc-DEF_123\r\n";
        assert_eq!(parse(text), Ok(Config { host: "192.168.1.14".into(), port: 7878, token: "abc-DEF_123".into() }));
    }

    #[test]
    fn parse_missingOrWrongLines_sayWhich() {
        assert_eq!(parse("port=7878\ntoken=t"), Err("It has no host= line.".into()));
        assert_eq!(parse("host=h\nport=\ntoken=t"), Err("It has no port= line.".into()));
        assert_eq!(parse("host=h\nport=78x\ntoken=t"), Err("Its port, 78x, is not a port number.".into()));
        assert_eq!(parse("host=h\nport=1"), Err("It has no token= line.".into()));
    }

    #[test]
    fn load_namesTheFile_whenMissingOrWrong() {
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(load(CONFIG_PATH, Err(missing)), Err("There is no /mnt/ext1/system/config/rado.cfg.".into()));
        assert_eq!(
            load(CONFIG_PATH, Ok("host=h".into())),
            Err("/mnt/ext1/system/config/rado.cfg can't be used. It has no port= line.".into())
        );
    }
}
