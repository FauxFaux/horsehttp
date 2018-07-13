use std::io;
use std::io::Read;
use std::io::Write;
use std::net;
use std::num;

use cast::u64;
use failure::Error;
use failure::ResultExt;
use httparse;
use httparse::EMPTY_HEADER;
use mime;
use multipart::server::Multipart;
use multipart::server::MultipartData;
use multipart::server::MultipartField;
use result::ResultOptionExt;

use req;

pub struct Client {
    requested: Requested,
    addr: net::SocketAddr,
    stream: net::TcpStream,
    response: Response,
}

pub struct Requested {
    method: String,
    path: String,
    version: u8,
    headers: Vec<(String, String)>,
    body_start: Vec<u8>,
}

pub struct Response {
    code: u16,
    message: String,
    pub sent: bool,
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

impl Client {
    pub(crate) fn new(
        requested: Requested,
        addr: net::SocketAddr,
        stream: net::TcpStream,
    ) -> Client {
        Client {
            requested,
            addr,
            stream,
            response: Response::default(),
        }
    }

    pub fn send_response(&mut self) -> Result<(), Error> {
        ensure!(!self.response.sent, "response already sent");
        self.write_response()?;
        Ok(())
    }

    pub fn set_response<S: Into<String>>(&mut self, code: u16, message: S) -> Result<(), Error> {
        ensure!(!self.response_sent(), "response already sent");
        let message = message.into();
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

    pub fn response_sent(&self) -> bool {
        self.response.sent
    }

    /// Do a raw write to the client.
    ///
    /// If headers haven't been sent, the server won't send them, now or ever.
    pub fn write_all_overriding_headers(&mut self, buf: &[u8]) -> Result<(), Error> {
        self.response.sent = true;
        self.stream.write_all(buf)?;
        Ok(())
    }

    pub fn request_header<S: AsRef<str>>(&self, name: S) -> Option<String> {
        let name = name.as_ref();
        match self
            .requested
            .headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, val)| val.to_string())
            .collect::<Vec<String>>()
        {
            ref v if v.is_empty() => None,
            v => Some(v.join(", ")),
        }
    }

    pub fn content_length(&self) -> Result<Option<usize>, num::ParseIntError> {
        self.request_header("Content-Length")
            .map(|len| len.parse())
            .invert()
    }

    pub fn body_reader<'a>(&'a mut self) -> Result<BodyReader<'a>, Error> {
        let len = self
            .content_length()?
            .ok_or_else(|| format_err!("no content length"))?;
        Ok(BodyReader {
            inner: self.take(u64(len)),
        })
    }

    pub fn body_parser(&mut self) -> Result<BodyParser, Error> {
        let content_type: mime::Mime = self
            .request_header("Content-Type")
            .ok_or_else(|| format_err!("POST must have content type"))?
            .parse()?;
        Ok(match (content_type.type_(), content_type.subtype()) {
            (mime::MULTIPART, mime::FORM_DATA) => {
                BodyParser::Form(Form::new(Multipart::with_body(
                    self.body_reader()?,
                    content_type
                        .get_param(mime::BOUNDARY)
                        .ok_or_else(|| format_err!("form-data but no boundary"))?
                        .as_ref(),
                )))
            }
            _ => BodyParser::Unknown(content_type, self.body_reader()?),
        })
    }

    fn send_response_if_not_already_sent(&mut self) -> io::Result<()> {
        if self.response_sent() {
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

pub enum BodyParser<'c> {
    Form(Form<'c>),
    Unknown(mime::Mime, BodyReader<'c>),
}

pub struct Form<'c> {
    multipass: Multipart<BodyReader<'c>>,
}

pub struct FormField<'c: 'f, 'f> {
    inner: MultipartField<&'f mut Multipart<BodyReader<'c>>>,
}

impl<'c: 'f, 'f> FormField<'c, 'f> {
    pub fn name(&self) -> String {
        self.inner.headers.name.to_string()
    }

    pub fn content_type(&self) -> Option<mime::Mime> {
        self.inner
            .headers
            .content_type
            .as_ref()
            // TODO: this is caused by multipart 0.15 using mime 0.2, 'cos hyper.
            // TODO: shouldn't need to go via a string and parse if they fix that,
            // TODO: or we downgrade mime. Downgrading mime isn't great, as the name
            // TODO: of everything has changed. Also, it's the future.
            .and_then(|m| m.to_string().parse().ok())
    }

    pub fn filename(&self) -> Option<String> {
        self.inner.headers.filename.clone()
    }

    pub fn data(&mut self) -> &mut MultipartData<&'f mut Multipart<BodyReader<'c>>> {
        &mut self.inner.data
    }
}

impl<'c> Form<'c> {
    fn new(multipass: Multipart<BodyReader<'c>>) -> Form<'c> {
        Form { multipass }
    }

    pub fn for_each<F>(&mut self, mut callback: F) -> Result<(), Error>
    where
        F: FnMut(FormField) -> Result<(), Error>,
    {
        while let Some(inner) = self.multipass.read_entry()? {
            callback(FormField { inner })?;
        }
        Ok(())
    }
}

pub struct BodyReader<'c> {
    inner: io::Take<&'c mut Client>,
}

impl<'c> Read for BodyReader<'c> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Read for Client {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        if self.requested.body_start.is_empty() {
            return self.stream.read(buf);
        }

        let to_reply = buf.len().min(self.requested.body_start.len());
        buf.copy_from_slice(&self.requested.body_start[..to_reply]);
        let _ = self.requested.body_start.drain(..to_reply);
        Ok(to_reply)
    }
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

pub(crate) fn parse_request(stream: &mut net::TcpStream) -> Result<Requested, Error> {
    let (header_block, body_start, headers) = req::read_headers(stream)?;
    let mut headers = vec![EMPTY_HEADER; headers];
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
