//! Load one complete face from a TTC without reading or cloning the other faces.
//! Table offsets in collections are absolute; table-internal offsets stay intact.
use std::io::{self, Read, Seek, SeekFrom};

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid font collection")
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes.chunks(4).fold(0, |sum, chunk| {
        let mut word = [0; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

pub fn load(
    reader: &mut (impl Read + Seek),
    file_len: u64,
    index: u32,
) -> io::Result<(Vec<u8>, u32)> {
    let mut header = [0; 12];
    reader.read_exact(&mut header)?;
    if &header[..4] != b"ttcf" {
        reader.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        return Ok((bytes, index));
    }
    if !matches!(u32_at(&header, 4), 0x0001_0000 | 0x0002_0000) || index >= u32_at(&header, 8) {
        return Err(invalid());
    }
    reader.seek(SeekFrom::Start(12 + u64::from(index) * 4))?;
    let mut offset = [0; 4];
    reader.read_exact(&mut offset)?;
    reader.seek(SeekFrom::Start(u64::from(u32::from_be_bytes(offset))))?;
    reader.read_exact(&mut header)?;
    if !matches!(&header[..4], b"OTTO" | b"\0\x01\0\0" | b"true") {
        return Err(invalid());
    }
    let count = usize::from(u16_at(&header, 4));
    // searchRange is a uint16 in units of 16 bytes.
    if count == 0 || count > 4095 {
        return Err(invalid());
    }
    let mut records = vec![0; count * 16];
    reader.read_exact(&mut records)?;
    let mut tables = Vec::with_capacity(count);
    for record in records.as_chunks::<16>().0 {
        let tag: [u8; 4] = record[..4].try_into().unwrap();
        let offset = u32_at(record, 8);
        let length = u32_at(record, 12);
        if u64::from(offset) + u64::from(length) > file_len {
            return Err(invalid());
        }
        // A digital signature of the original collection cannot sign the rebuilt face.
        if &tag != b"DSIG" {
            tables.push((tag, offset, length));
        }
    }
    tables.sort_unstable_by_key(|table| table.0);
    if tables.is_empty() || tables.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(invalid());
    }
    let directory_size = 12 + tables.len() * 16;
    let total = tables
        .iter()
        .try_fold(directory_size, |size, table| {
            size.checked_add((table.2 as usize).checked_add(3)? & !3)
        })
        .ok_or_else(invalid)?;
    // Refuse overlapping/bogus records that could amplify a small input into a huge allocation.
    if total as u64 > file_len || total > u32::MAX as usize {
        return Err(invalid());
    }
    let mut output = Vec::new();
    output.try_reserve_exact(total).map_err(io::Error::other)?;
    output.resize(directory_size, 0);
    output[..4].copy_from_slice(&header[..4]);
    let count = tables.len() as u16;
    let selector = count.ilog2() as u16;
    let search = (1u16 << selector) * 16;
    output[4..6].copy_from_slice(&count.to_be_bytes());
    output[6..8].copy_from_slice(&search.to_be_bytes());
    output[8..10].copy_from_slice(&selector.to_be_bytes());
    output[10..12].copy_from_slice(&(count * 16 - search).to_be_bytes());
    let mut head_adjustment = None;
    for (i, (tag, offset, length)) in tables.into_iter().enumerate() {
        let start = output.len();
        let end = start + length as usize;
        output.resize((end + 3) & !3, 0);
        reader.seek(SeekFrom::Start(u64::from(offset)))?;
        reader.read_exact(&mut output[start..end])?;
        if &tag == b"head" {
            if length < 12 {
                return Err(invalid());
            }
            output[start + 8..start + 12].fill(0);
            head_adjustment = Some(start + 8);
        }
        let sum = checksum(&output[start..end]);
        let record = &mut output[12 + i * 16..12 + (i + 1) * 16];
        record[..4].copy_from_slice(&tag);
        record[4..8].copy_from_slice(&sum.to_be_bytes());
        record[8..12].copy_from_slice(&(start as u32).to_be_bytes());
        record[12..16].copy_from_slice(&length.to_be_bytes());
    }
    let adjustment = head_adjustment.ok_or_else(invalid)?;
    let sum = 0xB1B0_AFBAu32.wrapping_sub(checksum(&output));
    output[adjustment..adjustment + 4].copy_from_slice(&sum.to_be_bytes());
    Ok((output, 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn collection() -> Vec<u8> {
        let mut bytes = vec![0; 204];
        bytes[..4].copy_from_slice(b"ttcf");
        bytes[4..8].copy_from_slice(&0x0002_0000u32.to_be_bytes());
        bytes[8..12].copy_from_slice(&2u32.to_be_bytes());
        for (i, directory) in [32u32, 92].into_iter().enumerate() {
            bytes[12 + i * 4..16 + i * 4].copy_from_slice(&directory.to_be_bytes());
            let start = directory as usize;
            bytes[start..start + 4].copy_from_slice(b"OTTO");
            bytes[start + 4..start + 6].copy_from_slice(&3u16.to_be_bytes());
            for (j, (tag, offset, length)) in [
                (*b"CFF ", 152 + i as u32 * 8, 5u32),
                (*b"DSIG", 200, 4),
                (*b"head", 168 + i as u32 * 16, 16),
            ]
            .into_iter()
            .enumerate()
            {
                let r = start + 12 + j * 16;
                bytes[r..r + 4].copy_from_slice(&tag);
                bytes[r + 8..r + 12].copy_from_slice(&offset.to_be_bytes());
                bytes[r + 12..r + 16].copy_from_slice(&length.to_be_bytes());
            }
        }
        bytes[152..157].copy_from_slice(b"first");
        bytes[160..165].copy_from_slice(b"other");
        bytes[168..184].fill(7);
        bytes[184..200].fill(9);
        bytes
    }

    #[test]
    fn selected_face_keeps_tables_and_rebuilds_offsets_and_checksums() {
        let bytes = collection();
        for (index, expected, head) in [(0, b"first", 7), (1, b"other", 9)] {
            let (font, face) = load(&mut Cursor::new(&bytes), bytes.len() as u64, index).unwrap();
            assert_eq!(face, 0);
            assert_eq!(&font[..4], b"OTTO");
            assert_eq!(u16_at(&font, 4), 2); // DSIG removed
            assert_eq!(&font[12..16], b"CFF ");
            let cff = u32_at(&font, 20) as usize;
            assert_eq!(&font[cff..cff + 5], expected);
            assert_eq!(u32_at(&font, 16), checksum(expected));
            let h = u32_at(&font, 36) as usize;
            assert_eq!(&font[h..h + 8], &[head; 8]);
            assert_eq!(&font[h + 12..h + 16], &[head; 4]);
            let mut table = font[h..h + 16].to_vec();
            table[8..12].fill(0);
            assert_eq!(u32_at(&font, 32), checksum(&table));
            assert_eq!(checksum(&font), 0xB1B0_AFBA);
            assert_eq!(cff % 4, 0);
            assert_eq!(h % 4, 0);
        }
    }

    #[test]
    fn rejects_missing_faces_and_out_of_bounds_tables() {
        let bytes = collection();
        assert!(load(&mut Cursor::new(&bytes), bytes.len() as u64, 2).is_err());
        for length in [0, 11, 45, 150, 190] {
            assert!(load(&mut Cursor::new(&bytes[..length]), length as u64, 0).is_err());
        }
        let mut bad = bytes.clone();
        bad[52..56].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(load(&mut Cursor::new(&bad), bad.len() as u64, 0).is_err());
    }

    #[test]
    fn standalone_fonts_pass_through_unchanged() {
        let bytes = b"OTTOstandalone font contents";
        let (font, index) = load(&mut Cursor::new(bytes), bytes.len() as u64, 0).unwrap();
        assert_eq!(&font, bytes);
        assert_eq!(index, 0);
    }

    #[test]
    fn rejects_duplicate_tables_and_allocation_amplification() {
        let mut bytes = collection();
        bytes[60..64].copy_from_slice(b"CFF ");
        assert!(load(&mut Cursor::new(&bytes), bytes.len() as u64, 0).is_err());
        let mut bytes = collection();
        let length = bytes.len() as u32;
        for record in [44, 76] {
            bytes[record + 8..record + 12].copy_from_slice(&0u32.to_be_bytes());
            bytes[record + 12..record + 16].copy_from_slice(&length.to_be_bytes());
        }
        assert!(load(&mut Cursor::new(&bytes), bytes.len() as u64, 0).is_err());
    }
}
