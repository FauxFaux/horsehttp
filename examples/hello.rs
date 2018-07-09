extern crate failure;
extern crate log;
extern crate pretty_env_logger;
extern crate worsehttp;

use std::io::Write;

use failure::Error;
use worsehttp::Client;
use worsehttp::HttpRequestHandler;

struct Handler {}

impl HttpRequestHandler for Handler {
    fn do_get(&mut self, client: &mut Client) -> Result<(), Error> {
        match client.path().as_str() {
            "/" => {
                let whom = client.addr();
                writeln!(client, "hello {} on port {}", whom.ip(), whom.port())?;
            }
            other => {
                client.set_response(404, "Not Found")?;
                writeln!(client, "I don't recognise the url {}\n", other)?;
            }
        }

        Ok(())
    }
}

fn main() -> Result<(), Error> {
    pretty_env_logger::formatted_builder()?
        .filter_level(log::LevelFilter::Debug)
        .init();
    worsehttp::serve(1337, |_| Handler {})
}
