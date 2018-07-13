#![feature(bufreader_buffer)]

extern crate cast;
#[macro_use]
extern crate failure;
extern crate httparse;
#[macro_use]
extern crate log;
extern crate mime;
extern crate multipart;
extern crate net2;
extern crate result;

mod client;
mod req;
mod semaphore;

use std::io::Write;
use std::net;
use std::panic;
use std::sync::Arc;
use std::thread;

use failure::Error;

pub use client::BodyParser;
pub use client::Client;

pub trait HttpRequestHandler: Send + panic::UnwindSafe {
    fn before(
        &mut self,
        _stream: &mut net::TcpStream,
        addr: &mut net::SocketAddr,
    ) -> Result<(), Error> {
        info!("{}: accepted connection", addr);
        Ok(())
    }

    fn handle(&mut self, client: &mut Client) -> Result<(), Error> {
        match client.method().as_str() {
            "GET" => self.do_get(client),
            "HEAD" => self.do_head(client),
            "POST" => self.do_post(client),
            _ => {
                client.set_response(405, "Method Not Allowed")?;
                Ok(())
            }
        }
    }

    fn do_get(&mut self, client: &mut Client) -> Result<(), Error> {
        client.set_response(405, "Method Not Allowed")
    }

    fn do_head(&mut self, client: &mut Client) -> Result<(), Error> {
        client.set_response(405, "Method Not Allowed")
    }

    fn do_post(&mut self, client: &mut Client) -> Result<(), Error> {
        client.set_response(405, "Method Not Allowed")
    }
}

pub fn serve<F, H>(port: u16, mut handler: F) -> Result<(), Error>
where
    F: FnMut(&net::SocketAddr) -> H,
    H: HttpRequestHandler + panic::UnwindSafe + 'static,
{
    let listen = net2::TcpBuilder::new_v4()?
        .reuse_address(true)?
        .bind(net::SocketAddr::from(([127, 0, 0, 1], port)))?
        .listen(64)?;

    info!("server listening on port {}", port);

    let open_connections = Arc::new(semaphore::Semaphore::new(4));

    loop {
        let (stream, addr) = listen.accept()?;

        open_connections.acquire();
        let permits = open_connections.clone();

        let handler = handler(&addr);
        thread::spawn(move || {
            if let Err(e) = panic::catch_unwind(move || {
                if let Err(e) = handle(stream, addr, handler) {
                    error!("error handling request from {}: {}", addr, e);
                }
            }) {
                error!("fatal error handling request from {}: {:?}", addr, e);
            }

            permits.release();
        });
    }
}

fn handle(
    mut stream: net::TcpStream,
    mut addr: net::SocketAddr,
    mut handler: impl HttpRequestHandler,
) -> Result<(), Error> {
    handler.before(&mut stream, &mut addr)?;

    let requested = match client::parse_request(&mut stream) {
        Ok(requested) => requested,
        Err(e) => {
            warn!("bad request from {}: {:?}", addr, e);
            stream.write_all(
                b"HTTP/1.0 400 Bad Request\r\nConnection: close\r\n\r\nerr: bad request\r\n",
            )?;
            return Ok(());
        }
    };

    let mut client = Client::new(requested, addr, stream);

    let status = {
        // TODO: Not sure about this `AssertUnwindSafe`; we're asserting that the `&mut` is valid,
        // TODO: as `Client` itself already is. Code using `Client` after this point should probably
        // TODO: be careful. But, also, what's going to happen? It's not unsafe, the worst is
        // TODO: presumably a further panic, which we'll see in the upper error handling anyway.
        let unwind_client = panic::AssertUnwindSafe(&mut client);
        match panic::catch_unwind(move || handler.handle(unwind_client.0)) {
            Ok(Ok(())) => None,
            Ok(Err(err)) => Some(err),
            Err(any) => Some(format_err!(
                "panic: {}",
                any.downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| any.downcast_ref::<String>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "Box<Any>".to_string())
            )),
        }
    };

    if !client.response_sent() {
        if let Some(e) = status {
            error!("{}: returning 500 for: {}", client.addr(), e);
            client.set_response(500, "Internal Server Error")?;
            client.write_all(b"err: internal")?;
        } else {
            client.send_response()?;
            info!(
                "{}: finished successfully, backend sent response",
                client.addr()
            );
        }
    } else {
        if let Some(e) = status {
            error!("{}: error after headers sent: {}", client.addr(), e);
        } else {
            info!(
                "{}: finished successfully, user sent response",
                client.addr()
            );
        }
    }

    Ok(())
}
