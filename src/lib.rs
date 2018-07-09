#![feature(bufreader_buffer)]

#[macro_use]
extern crate failure;
extern crate httparse;
#[macro_use]
extern crate log;
extern crate net2;

mod req;

use std::io;
use std::io::Write;
use std::net;
use std::panic;
use std::thread;

use failure::Error;
use failure::ResultExt;

pub trait HttpRequestHandler: Send {
    fn before(
        &mut self,
        stream: &mut net::TcpStream,
        addr: &mut net::SocketAddr,
    ) -> Result<(), Error> {
        info!("{}: accepted connection", addr);
        Ok(())
    }

    fn handle(&mut self, client: &mut Client) -> Result<(), Error> {
        match client.method().as_str() {
            "GET" => self.do_get(client),
            "HEAD" => self.do_head(client),
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
}

struct Requested {
    method: String,
    path: String,
    version: u8,
    headers: Vec<(String, String)>,
    body_start: Vec<u8>,
}

struct Response {
    code: u16,
    message: String,
    sent: bool,
}

impl Default for Response {
    fn default() -> Self {
        Response {
            code: 200,
            message: "Ok".to_string(),
            sent: false,
        }
    }
}

pub struct Client {
    requested: Requested,
    addr: net::SocketAddr,
    stream: net::TcpStream,
    response: Response,
}

impl Write for Client {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.send_response_if_not_already_sent()?;
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.send_response_if_not_already_sent()?;
        self.stream.write_all(buf)
    }
}

impl Client {
    pub fn send_response(&mut self) -> Result<(), Error> {
        ensure!(!self.response.sent, "response already sent");
        self.write_response()?;
        Ok(())
    }

    pub fn set_response<S: ToString>(&mut self, code: u16, message: S) -> Result<(), Error> {
        ensure!(!self.response.sent, "response already sent");
        let message = message.to_string();
        ensure!(
            !message.contains(|c: char| c.is_ascii_control()),
            "header line shouldn't contain control characters"
        );
        self.response.code = code;
        self.response.message = message;

        Ok(())
    }

    pub fn addr(&self) -> net::SocketAddr {
        self.addr.clone()
    }

    pub fn method(&self) -> String {
        self.requested.method.to_string()
    }

    pub fn path(&self) -> String {
        self.requested.path.to_string()
    }

    fn send_response_if_not_already_sent(&mut self) -> io::Result<()> {
        if self.response.sent {
            return Ok(());
        }

        self.write_response()
    }

    fn write_response(&mut self) -> io::Result<()> {
        self.response.sent = true;

        write!(
            self.stream,
            "HTTP/1.{} {} {}\r\n",
            self.requested.version, self.response.code, self.response.message
        )?;
        // TODO: headers
        write!(self.stream, "Connection: close\r\n\r\n")?;
        info!("{}: sent {}", self.addr, self.response.code);
        Ok(())
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

    loop {
        let (stream, addr) = listen.accept()?;

        let handler = handler(&addr);
        thread::spawn(move || {
            if let Err(e) = panic::catch_unwind(move || {
                if let Err(e) = handle(stream, addr, handler) {
                    error!("error handling request from {}: {}", addr, e);
                }
            }) {
                error!("fatal error handling request from {}: {:?}", addr, e);
            }
        });
    }
}

pub fn handle(
    mut stream: net::TcpStream,
    mut addr: net::SocketAddr,
    mut handler: impl HttpRequestHandler,
) -> Result<(), Error> {
    handler.before(&mut stream, &mut addr)?;

    let requested = match parse_request(&mut stream) {
        Ok(requested) => requested,
        Err(e) => {
            warn!("bad request from {}: {:?}", addr, e);
            stream.write_all(
                b"HTTP/1.0 400 Bad Request\r\nConnection: close\r\n\r\nerr: bad request\r\n",
            )?;
            return Ok(());
        }
    };

    let mut client = Client {
        requested,
        addr,
        stream,
        response: Response::default(),
    };

    let status = handler.handle(&mut client);

    if !client.response.sent {
        if let Err(e) = status {
            error!("handling: {}", e);
            client.stream.write_all(b"HTTP/1.{} 500 Internal Server Error\r\nConnection: close\r\n\r\nerr: internal\r\n")?;
            return Ok(());
        }

        client.send_response()?;
    }

    info!("{}: finished successfully", client.addr);

    Ok(())
}

fn parse_request(stream: &mut net::TcpStream) -> Result<Requested, Error> {
    let (header_block, body_start, headers) = req::read_headers(stream)?;
    let mut headers = vec![httparse::EMPTY_HEADER; headers];
    let mut request = httparse::Request::new(&mut headers);
    request.parse(&header_block)?;
    Ok(Requested {
        version: request
            .version
            .ok_or_else(|| format_err!("no http version in request"))?,
        path: request
            .path
            .ok_or_else(|| format_err!("no http path in request"))?
            .to_string(),
        method: request
            .method
            .ok_or_else(|| format_err!("no http method in request"))?
            .to_string(),
        headers: request
            .headers
            .into_iter()
            .map(|h| String::from_utf8(h.value.to_vec()).map(|value| (h.name.to_string(), value)))
            .collect::<Result<Vec<(String, String)>, ::std::string::FromUtf8Error>>()
            .with_context(|_| format_err!("decoding header values"))?,
        body_start,
    })
}
