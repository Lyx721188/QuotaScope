//! Stream DSH frames without invoking a program or allocating the transcript.
//! Raw/decoded ceilings and the decoder's history window are independent.
use std::io::{self, Read};
pub(crate) const MAX_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

struct Limited<R> {
    inner: R,
    remaining: u64,
}
impl<R: Read> Read for Limited<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if !crate::scan::checkpoint() {
            return Err(io::Error::other("scan cancelled"));
        }
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut extra = [0u8; 1];
            return if self.inner.read(&mut extra)? == 0 {
                Ok(0)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "usage stream exceeds the read limit",
                ))
            };
        }
        let maximum = self.remaining.min(buffer.len() as u64) as usize;
        let count = self.inner.read(&mut buffer[..maximum])?;
        self.remaining -= count as u64;
        Ok(count)
    }
}
pub(crate) fn decode(reader: impl Read + 'static) -> io::Result<Box<dyn Read>> {
    decode_limits(reader, MAX_BYTES, MAX_BYTES)
}
pub(crate) fn plain(reader: impl Read + 'static) -> Box<dyn Read> {
    plain_limit(reader, MAX_BYTES)
}
pub(crate) fn plain_limit(reader: impl Read + 'static, maximum: u64) -> Box<dyn Read> {
    Box::new(Limited {
        inner: reader,
        remaining: maximum,
    })
}
fn decode_limits(reader: impl Read + 'static, raw: u64, decoded: u64) -> io::Result<Box<dyn Read>> {
    let mut decoder = zstd::stream::read::Decoder::new(Limited {
        inner: reader,
        remaining: raw,
    })?;
    // Eight MiB of back references; oversized frames stay explicitly partial.
    decoder.window_log_max(23)?;
    // The default decoder reads every concatenated frame and reports a torn
    // tail. Do not use single_frame() or turn exhausted limits into clean EOF.
    Ok(Box::new(Limited {
        inner: decoder,
        remaining: decoded,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concatenated_frames_and_exact_limits_finish_but_excess_is_an_error() {
        let mut encoded = zstd::stream::encode_all(&b"first\n"[..], 1).unwrap();
        encoded.extend(zstd::stream::encode_all(&b"second\n"[..], 1).unwrap());
        let length = encoded.len() as u64;
        let mut output = String::new();
        decode_limits(std::io::Cursor::new(encoded.clone()), length, 13)
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        assert_eq!(output, "first\nsecond\n");
        assert!(
            decode_limits(std::io::Cursor::new(encoded.clone()), length - 1, 13)
                .unwrap()
                .read_to_end(&mut Vec::new())
                .is_err()
        );
        assert!(decode_limits(std::io::Cursor::new(encoded), length, 12)
            .unwrap()
            .read_to_end(&mut Vec::new())
            .is_err());
    }
    #[test]
    fn truncated_or_corrupt_frames_never_look_like_clean_eof() {
        let encoded = zstd::stream::encode_all(&b"some complete records\n"[..], 1).unwrap();
        for bytes in [
            encoded[..encoded.len() - 1].to_vec(),
            vec![0x28, 0xb5, 0x2f, 0xfd, 0xff, 0xff],
        ] {
            assert!(decode(std::io::Cursor::new(bytes))
                .unwrap()
                .read_to_end(&mut Vec::new())
                .is_err());
        }
    }
    #[test]
    fn a_large_decoder_window_is_rejected_before_allocating_it() {
        let mut encoder = zstd::stream::Encoder::new(Vec::new(), 1).unwrap();
        encoder.window_log(24).unwrap();
        encoder.write_all(&vec![b'a'; 16 * 1024 * 1024]).unwrap();
        let bytes = encoder.finish().unwrap();
        assert!(decode(std::io::Cursor::new(bytes))
            .unwrap()
            .read_to_end(&mut Vec::new())
            .is_err());
    }
    use std::io::Write;
}
