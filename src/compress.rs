// Compression by NobodyNada, with some small tweaks by Maddo,
// to optimize a bit more for decompression speed compared to space.
use crate::{Command, Reference};

/// Compresses the provided data.
pub fn compress(src: &[u8]) -> Vec<u8> {
    let mut dst = Vec::new();

    let mut i = 0;
    let mut prev_copy = Vec::new();
    while i < src.len() {
        let best = find_best(src, i);
        // We consider that the new command has to save at least 3 bytes to be worthwhile over a copy.
        // It could save space with only 2 (or possibly 1) byte, but decompression will
        // be faster by using a larger copy block.
        if best.len() >= best.cost() + 3 {
            if !prev_copy.is_empty() {
                Command::Copy(&prev_copy[..]).write(&mut dst);
                prev_copy = Vec::new();
            }
            best.write(&mut dst);
            i += best.len();
        } else {
            prev_copy.push(src[i]);
            i += 1;
        }
    }

    if !prev_copy.is_empty() {
        Command::Copy(&prev_copy[..]).write(&mut dst);
    }

    Command::Stop.write(&mut dst);

    dst
}

fn get_candidates(src: &[u8], i: usize) -> Vec<Command> {
    let mut candidates = vec![];

    if src.len() - i >= 2 {
        let word = u16::from_le_bytes([src[i], src[i + 1]]);
        let mut len = src[i..]
            .chunks_exact(2)
            .take_while(|c| u16::from_le_bytes((*c).try_into().unwrap()) == word)
            .count()
            * 2;

        // A word fill can have a partial last word
        if src.get(i + len).copied() == Some(word as u8) {
            len += 1;
        }

        let len = std::cmp::min(len, Command::MAX_LEN);
        candidates.push(Command::WordFill { data: word, len });
        if len == Command::MAX_LEN {
            // Skip considering other block types if this is a max-size block:
            // This can speed up compression significantly, because large
            // blocks of repeated data would trigger worst-case slow behavior
            // in the backreference search.
            return candidates;
        }
    }

    candidates.push(Command::ByteFill {
        data: src[i],
        len: std::cmp::min(
            src[i..].iter().take_while(|&&x| x == src[i]).count(),
            Command::MAX_LEN,
        ),
    });

    candidates.push(Command::Incrementing {
        start: src[i],
        len: std::cmp::min(
            std::iter::zip(
                std::iter::successors(Some(src[i]), |x| Some(x.wrapping_add(1))),
                src[i..].iter().copied(),
            )
            .take_while(|(a, b)| a == b)
            .count(),
            Command::MAX_LEN,
        ),
    });

    if let Some(cand) = find_best_backreference(src, i) {
        candidates.push(cand);
    }

    candidates
}

fn find_best(src: &[u8], i: usize) -> Command {
    let mut candidates = get_candidates(src, i);
    
    // We want to prioritize earlier candidates in case of ties, but max_by prioritizes last.
    // So reverse the order:
    candidates.reverse();

    candidates
        .into_iter()
        .max_by(|a, b| {
            let a = a.len() as f32 / a.cost() as f32;
            let b = b.len() as f32 / b.cost() as f32;
            a.partial_cmp(&b).unwrap()
        })
        .unwrap()
}

fn find_best_backreference(src: &[u8], i: usize) -> Option<Command> {
    let mut best_relative = (0, false, 0); // a (j, inv, len) pair
    let farthest_relative = i - std::cmp::min(i, 255);
    for j in farthest_relative..i {
        let (inv, mut len) = backreference_at(src, i, j);
        if inv {
            // Maximum length for an inverted relative backreference is 0x300
            // due to collision with stop command
            len = len.min(0x300);
        }
        // if all else is equal, non-inverted relative matches save a byte (because relative
        // inverted can only be encoded as an extended command)
        if len > best_relative.2 || len == best_relative.2 && !inv && best_relative.1 {
            best_relative = (j, inv, len);
        }
    }

    let mut best_absolute = (0, false, 0); // a (j, inv, len) pair
    for j in 0..std::cmp::min(farthest_relative, (u16::MAX as usize) + 1) {
        let (inv, len) = backreference_at(src, i, j);
        if len > best_absolute.2 {
            best_absolute = (j, inv, len);
        }
    }

    match (best_relative, best_absolute) {
        // No match found
        ((_, _, 0), (_, _, 0)) => None,

        // Relative match is best
        ((j, invert, len), abs) if len >= abs.2 => Some(Command::Backreference {
            src: Reference::Relative((i - j).try_into().unwrap()),
            invert,
            len,
        }),

        // Absolute is best
        (_, (j, invert, len)) => Some(Command::Backreference {
            src: Reference::Absolute(j.try_into().unwrap()),
            invert,
            len,
        }),
    }
}

fn backreference_at(src: &[u8], i: usize, j: usize) -> (bool, usize) {
    let len = std::iter::zip(src[i..].iter().copied(), src[j..].iter().copied())
        .take_while(|(a, b)| *a == *b )
        .count();
    let len = std::cmp::min(len, Command::MAX_LEN);
    if len > 0 {
        return (false, len);
    }
    (false, 0)
}

impl Command<'_> {
    fn len(&self) -> usize {
        match self {
            Command::Copy(buf) => buf.len(),
            Command::ByteFill { data: _, len } => *len,
            Command::WordFill { data: _, len } => *len,
            Command::Incrementing { start: _, len } => *len,
            Command::Backreference {
                src: _,
                invert: _,
                len,
            } => *len,
            Command::Stop => 0,
        }
    }

    fn cost(&self) -> usize {
        // Includes tweaks to assign higher costs to block types
        // that are slower to decompress:
        let args = match self {
            Command::Copy(buf) => buf.len(),
            Command::ByteFill { data: _, len: _ } => 1,
            Command::WordFill { data: _, len: _ } => 2,
            Command::Incrementing { start: _, len: _ } => 2,
            Command::Backreference {
                src: Reference::Relative(_),
                invert: _,
                len: _,
            } => 3,
            Command::Backreference {
                src: _,
                invert: _,
                len: _,
            } => 4,
            Command::Stop => 0,
        };

        if self.len() <= 32 {
            args + 1
        } else {
            args + 2
        }
    }

    fn write(&self, dst: &mut Vec<u8>) {
        fn _write(cmd: u8, len: usize, data: &[u8], dst: &mut Vec<u8>) {
            let len = len - 1;
            if len < 32 && cmd != 7 {
                dst.push((cmd << 5) | len as u8);
            } else {
                assert!(len < Command::MAX_LEN);
                dst.push(0xE0 | (cmd << 2) | (len >> 8) as u8);
                dst.push(len as u8);
            }

            dst.extend_from_slice(data);
        }
        match self {
            Command::Copy(data) => _write(0, self.len(), data, dst),
            Command::ByteFill { data, len } => _write(1, *len, &[*data], dst),
            Command::WordFill { data, len } => _write(2, *len, &data.to_le_bytes(), dst),
            Command::Incrementing { start, len } => _write(3, *len, &[*start], dst),
            Command::Backreference { src, invert, len } => {
                match src {
                    Reference::Absolute(addr) => {
                        _write(4 | *invert as u8, *len, &addr.to_le_bytes(), dst)
                    }
                    Reference::Relative(offset) => {
                        assert_ne!(*offset, 0);
                        if *invert {
                            assert!(*len <= 0x300);
                        }
                        _write(6 | *invert as u8, *len, &[*offset], dst)
                    }
                };
            }

            Command::Stop => dst.push(0xFF),
        };
    }
}
