#[macro_use]
extern crate failure;
extern crate horsehttp;
#[macro_use]
extern crate log;
extern crate mime;
extern crate pretty_env_logger;

use std::io::Read;
use std::io::Write;

use failure::Error;
use horsehttp::BodyParser;
use horsehttp::Client;
use horsehttp::HttpRequestHandler;

struct Handler {}

impl HttpRequestHandler for Handler {
    fn do_get(&mut self, client: &mut Client) -> Result<(), Error> {
        match client.path().as_str() {
            "/" => {
                let whom = client.addr();
                writeln!(client, "hello {} on port {}", whom.ip(), whom.port())?;
            }
            "/panic" => panic!("can do!"),
            other => {
                client.set_response(404, "Not Found")?;
                writeln!(client, "I don't recognise the url {}\n", other)?;
            }
        }

        Ok(())
    }

    fn do_post(&mut self, client: &mut Client) -> Result<(), Error> {
        match client.path().as_str() {
            "/save" => match client.body_parser()? {
                BodyParser::Form(mut form) => {
                    form.for_each(|mut entry| {
                        info!(
                            "form entry named {:?} of type {:?}",
                            entry.name(),
                            entry.content_type(),
                        );
                        if let Some(name) = entry.filename() {
                            info!(" - file client names: {:?}", name);
                        } else {
                            let mut buf = Vec::new();
                            entry.data().read_to_end(&mut buf)?;
                            info!(" - unparsed body: {:?}", String::from_utf8_lossy(&buf));
                        }
                        Ok(())
                    })?;
                }
                BodyParser::Unknown(mime, _reader) => {
                    info!("unknown body type: {:?}", mime);
                }
            },
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
        .filter_level(log::LevelFilter::Info)
        .init();
    horsehttp::serve(1337, |_| Handler {})
}
