use crate::{Command, Reference};
use thiserror::Error;

/// Decompresses the provided data.
pub fn decompress(mut src: &[u8]) -> Result<Vec<u8>, DecompressionError> {
    let mut dst = Vec::new();
    loop {
        match read_cmd(&mut src)? {
            Command::Copy(buf) => dst.extend_from_slice(buf),
            Command::ByteFill { data, len } => dst.extend(std::iter::repeat(data).take(len)),
            Command::WordFill { data, len } => {
                dst.extend(std::iter::repeat(data.to_le_bytes()).flatten().take(len))
            }
            Command::Incrementing { start, len } => dst
                .extend(std::iter::successors(Some(start), |x| Some(x.wrapping_add(1))).take(len)),
            Command::Backreference { src, invert, len } => {
                let start = match src {
                    Reference::Absolute(i) => i as usize,
                    Reference::Relative(i) => {
                        if (i as usize) <= dst.len() {
                            dst.len() - i as usize
                        } else {
                            return Err(DecompressionError::WindowOutOfRange);
                        }
                    }
                };

                if start >= dst.len() {
                    return Err(DecompressionError::WindowOutOfRange);
                }

                dst.reserve(len);
                for i in 0..len {
                    dst.push(dst[start + i] ^ if invert { 0xFF } else { 0 });
                }
            }

            Command::Stop => break,
        }
    }
    Ok(dst)
}

/// Errors that can occur during decompression.
#[derive(Error, Debug, PartialEq, Eq)]
pub enum DecompressionError {
    #[error("Unexpected end of input")]
    UnexpectedEof,

    #[error("Window start invalid")]
    WindowOutOfRange,
}

fn read_byte(src: &mut &[u8]) -> Result<u8, DecompressionError> {
    if !src.is_empty() {
        let result = src[0];
        *src = &src[1..];
        Ok(result)
    } else {
        Err(DecompressionError::UnexpectedEof)
    }
}

fn read_word(src: &mut &[u8]) -> Result<u16, DecompressionError> {
    Ok(u16::from_le_bytes([read_byte(src)?, read_byte(src)?]))
}

fn read_cmd<'a>(src: &mut &'a [u8]) -> Result<Command<'a>, DecompressionError> {
    let cmd = read_byte(src)?;
    if cmd == 0xFF {
        return Ok(Command::Stop);
    }

    let mut len = (cmd & 0x1f) as usize;
    let mut cmd = cmd >> 5;

    // Parse extended size
    if cmd == 0x7 {
        cmd = (len >> 2) as u8;

        let next = read_byte(src)?;
        len = ((len & 0x3) << 8) | next as usize;
    }

    let len = len + 1;

    match cmd {
        0x0 => {
            if len > src.len() {
                Err(DecompressionError::UnexpectedEof)
            } else {
                let (data, next) = src.split_at(len);
                *src = next;
                Ok(Command::Copy(data))
            }
        }
        0x1 => Ok(Command::ByteFill {
            data: read_byte(src)?,
            len,
        }),
        0x2 => Ok(Command::WordFill {
            data: read_word(src)?,
            len,
        }),
        0x3 => Ok(Command::Incrementing {
            start: read_byte(src)?,
            len,
        }),
        0x4..=0x7 => {
            let src = if cmd < 0x6 {
                Reference::Absolute(read_word(src)?)
            } else {
                Reference::Relative(read_byte(src)?)
            };
            let invert = (cmd & 0x1) != 0;
            Ok(Command::Backreference { src, invert, len })
        }
        0x8..=0xFF => unreachable!(),
    }
}
