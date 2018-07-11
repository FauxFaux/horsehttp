#[macro_use]
extern crate failure;
#[macro_use]
extern crate log;
extern crate mime;
extern crate multipart;
extern crate pretty_env_logger;
extern crate worsehttp;

use std::io::Read;
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

    fn do_post(&mut self, client: &mut Client) -> Result<(), Error> {
        match client.path().as_str() {
            "/save" => {
                let content_type: mime::Mime = client
                    .request_header("Content-Type")
                    .ok_or_else(|| format_err!("POST must have content type"))?
                    .parse()?;
                match (content_type.type_(), content_type.subtype()) {
                    (mime::MULTIPART, mime::FORM_DATA) => {
                        let mut multipass = multipart::server::Multipart::with_body(
                            client.body_reader()?,
                            content_type
                                .get_param(mime::BOUNDARY)
                                .ok_or_else(|| format_err!("form-data but no boudary"))?
                                .as_ref(),
                        );

                        while let Some(mut entry) = multipass.read_entry()? {
                            info!(
                                "form entry named {:?} of type {:?}",
                                entry.headers.name, entry.headers.content_type
                            );
                            if let Some(name) = entry.headers.filename {
                                info!(" - file client names: {:?}", name);
                            } else {
                                let mut buf = Vec::new();
                                entry.data.read_to_end(&mut buf)?;
                                info!(" - unparsed body: {:?}", String::from_utf8_lossy(&buf));
                            }
                        }
                    }
                    other => {
                        println!("unknown content type: {:?}", other);
                        let mut body = Vec::new();
                        client.body_reader()?.read_to_end(&mut body)?;
                        writeln!(client, "hello {:?}", String::from_utf8_lossy(&body))?;
                    }
                }
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
        .filter_level(log::LevelFilter::Info)
        .init();
    worsehttp::serve(1337, |_| Handler {})
}
