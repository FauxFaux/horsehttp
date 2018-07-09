use std::io;
use std::io::BufRead;
use std::io::Read;

use failure::Error;

/// @return: (header block, body start, header count)
pub fn read_headers<R: Read>(mut from: R) -> Result<(Vec<u8>, Vec<u8>, usize), Error> {
    // We've basically rewritten half of bufreader here, would it be easier not to use it?
    let mut from = io::BufReader::new(from);

    let mut ret = Vec::with_capacity(256);
    let mut lines = 0;
    loop {
        from.read_until(b'\n', &mut ret)?;
        lines += 1;
        assert!(ret.ends_with(b"\n"));
        if ret.ends_with(b"\n\r\n") || ret.ends_with(b"\n\n") {
            break;
        }
    }
    Ok((ret, from.buffer().to_vec(), lines))
}
