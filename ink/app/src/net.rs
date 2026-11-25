//! The network thread: asks Rado for the list, waits as `Backoff` says, and hands every outcome to the UI.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use ink_core::api::{self, Fetched};
use ink_core::backoff::Backoff;
use ink_core::config::Config;
use ink_core::model::Failure;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const IO_TIMEOUT: Duration = Duration::from_secs(10);

pub type Outcome = Result<Fetched, Failure>;

pub fn spawn(config: Config, tx: Sender<Outcome>) {
    thread::spawn(move || {
        let mut etag = None;
        let mut backoff = Backoff::default();
        loop {
            let outcome = fetch(&config, etag.as_deref());
            if let Ok(Fetched::Changed { etag: new, .. }) = &outcome {
                etag.clone_from(new);
            }
            let wait = if outcome.is_ok() { backoff.answered() } else { backoff.failed() };
            if tx.send(outcome).is_err() {
                return;
            }
            thread::sleep(wait);
        }
    });
}

fn fetch(config: &Config, etag: Option<&str>) -> Outcome {
    let unreachable = |_| Failure::Unreachable;
    let address = (config.host.as_str(), config.port).to_socket_addrs().map_err(unreachable)?.next().ok_or(Failure::Unreachable)?;
    let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).map_err(unreachable)?;
    stream.set_read_timeout(Some(IO_TIMEOUT)).map_err(unreachable)?;
    stream.set_write_timeout(Some(IO_TIMEOUT)).map_err(unreachable)?;
    stream.write_all(api::list_request(config, etag).as_bytes()).map_err(unreachable)?;
    // Rado answers with Connection: close, so the response ends where the stream does.
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).map_err(unreachable)?;
    api::parse_response(&raw).map_err(Failure::Answer)
}
