//! A small PNG encoder for the thumbnails the new look shows (the shell
//! gives 32-bit pixels; the page takes images only from the program's own
//! protocol, so they are sent as PNG). Platform independent and tested.

use std::io::Write;

/// PNG bytes of a `w`×`h` picture whose pixels are 32-bit `0xAARRGGBB`
/// (Windows' DIB order), top row first, with premultiplied alpha (as the
/// shell's bitmaps are); `None` for a size that doesn't match the pixels.
pub fn from_argb(w: u32, h: u32, px: &[u32]) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || w > 4096 || h > 4096 || px.len() != (w as usize) * (h as usize) {
        return None;
    }
    // Each row: filter byte 0, then RGBA.
    let mut raw = Vec::with_capacity((w as usize * 4 + 1) * h as usize);
    for row in px.chunks(w as usize) {
        raw.push(0);
        for &p in row {
            let a = (p >> 24) as u8;
            let un = |c: u32| -> u8 {
                let c = (c & 0xFF) as u16;
                if a == 0 { 0 } else { (c * 255 / a as u16).min(255) as u8 }
            };
            raw.extend_from_slice(&[un(p >> 16), un(p >> 8), un(p), a]);
        }
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    z.write_all(&raw).ok()?;
    let data = z.finish().ok()?;
    let mut out = Vec::with_capacity(data.len() + 64);
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA, deflate, no filter, no interlace
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &data);
    chunk(&mut out, b"IEND", &[]);
    Some(out)
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut crc = flate2::Crc::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc.sum().to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// The chunks of a PNG: (kind, data), each CRC checked.
    fn chunks(png: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let mut at = 8;
        let mut out = Vec::new();
        while at < png.len() {
            let n = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            let kind: [u8; 4] = png[at + 4..at + 8].try_into().unwrap();
            let data = png[at + 8..at + 8 + n].to_vec();
            let mut crc = flate2::Crc::new();
            crc.update(&kind);
            crc.update(&data);
            assert_eq!(u32::from_be_bytes(png[at + 8 + n..at + 12 + n].try_into().unwrap()), crc.sum());
            out.push((kind, data));
            at += 12 + n;
        }
        out
    }

    #[test]
    fn encodes_rgba_rows() {
        // Opaque red, half-transparent green (premultiplied), transparent.
        let px = [0xFFFF_0000, 0x8000_8000, 0x0000_0000, 0xFF00_00FF];
        let png = from_argb(2, 2, &px).unwrap();
        let c = chunks(&png);
        let kinds: Vec<&[u8]> = c.iter().map(|(k, _)| &k[..]).collect();
        assert_eq!(kinds, [&b"IHDR"[..], b"IDAT", b"IEND"]);
        assert_eq!(c[0].1, [0, 0, 0, 2, 0, 0, 0, 2, 8, 6, 0, 0, 0]);
        let mut raw = Vec::new();
        flate2::read::ZlibDecoder::new(&c[1].1[..]).read_to_end(&mut raw).unwrap();
        assert_eq!(raw, [0, 255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 0, 0, 0, 0, 0, 255, 255]);
    }

    #[test]
    fn refuses_odd_sizes() {
        assert_eq!(from_argb(0, 1, &[]), None);
        assert_eq!(from_argb(2, 2, &[0; 3]), None);
        assert_eq!(from_argb(5000, 1, &[0; 5000]), None);
    }
}
