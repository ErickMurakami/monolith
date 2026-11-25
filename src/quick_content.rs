//! Bounded, read-only preview decoding. Nothing here invokes a file handler.

use crate::artwork::{Artwork, MAX_STREAM};
use std::{
    fs::{self, File},
    io::Read,
    path::Path,
};
use windows::{
    Data::Pdf::{PdfDocument, PdfPageRenderOptions},
    Graphics::Imaging::BitmapDecoder,
    Storage::Streams::{DataWriter, IRandomAccessStream, InMemoryRandomAccessStream},
    Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
    core::{GUID, Interface},
};

const TEXT_LIMIT: u64 = 256 * 1024;
const IMAGE_SIDE: u32 = 1024;

pub enum Content {
    Directory,
    Details(u64),
    Text { value: String, cut: bool },
    Image(Artwork),
    Pdf { image: Artwork, pages: u32 },
    TooLarge,
    Unavailable,
}

pub fn read(path: &Path) -> Content {
    let Ok(metadata) = fs::metadata(path) else {
        return Content::Unavailable;
    };
    if metadata.is_dir() {
        return Content::Directory;
    }
    if !metadata.is_file() {
        return Content::Unavailable;
    }
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let text = matches!(
        extension.as_str(),
        "txt"
            | "md"
            | "markdown"
            | "log"
            | "csv"
            | "json"
            | "toml"
            | "ini"
            | "yaml"
            | "yml"
            | "xml"
            | "html"
            | "css"
            | "rs"
            | "py"
            | "js"
            | "ts"
            | "sh"
            | "ps1"
    );
    let image = matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp"
    );
    let pdf = extension == "pdf";
    if !(text || image || pdf) {
        return Content::Details(metadata.len());
    }
    let limit = if text { TEXT_LIMIT } else { MAX_STREAM };
    if !text && metadata.len() > limit {
        return Content::TooLarge;
    }
    let Ok(file) = File::open(path) else {
        return Content::Unavailable;
    };
    // Read the same inspected file handle, even if the path is replaced while loading.
    let Ok(opened) = file.metadata() else {
        return Content::Unavailable;
    };
    if !opened.is_file() {
        return Content::Unavailable;
    }
    let mut bytes = Vec::new();
    if file.take(limit + 1).read_to_end(&mut bytes).is_err() {
        return Content::Unavailable;
    }
    let cut = bytes.len() as u64 > limit;
    if text {
        bytes.truncate(limit as usize);
        return text_content(&bytes, cut).map_or(Content::Details(opened.len()), |value| {
            Content::Text { value, cut }
        });
    }
    if cut {
        return Content::TooLarge;
    }
    if pdf && !bytes.starts_with(b"%PDF-") {
        return Content::Unavailable;
    }
    if unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_err() {
        return Content::Unavailable;
    }
    let _apartment = Apartment;
    if pdf {
        return render_pdf(&bytes).unwrap_or(Content::Unavailable);
    }
    let picture = image_codec(&bytes).and_then(|codec| {
        memory_stream(&bytes)
            .and_then(|stream| Artwork::decode_stream(&stream, IMAGE_SIDE, Some(codec)))
    });
    picture.map_or(Content::Unavailable, Content::Image)
}

struct Apartment;

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

fn image_codec(bytes: &[u8]) -> Option<GUID> {
    match bytes {
        [0xff, 0xd8, 0xff, ..] => BitmapDecoder::JpegDecoderId(),
        [0x89, b'P', b'N', b'G', 13, 10, 0x1a, 10, ..] => BitmapDecoder::PngDecoderId(),
        [b'B', b'M', ..] => BitmapDecoder::BmpDecoderId(),
        [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => BitmapDecoder::GifDecoderId(),
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => BitmapDecoder::WebpDecoderId(),
        _ => return None,
    }
    .ok()
}

fn memory_stream(bytes: &[u8]) -> Option<IRandomAccessStream> {
    let stream = InMemoryRandomAccessStream::new().ok()?;
    let output = stream.GetOutputStreamAt(0).ok()?;
    let writer = DataWriter::CreateDataWriter(&output).ok()?;
    writer.WriteBytes(bytes).ok()?;
    let stored = writer.StoreAsync().ok()?.join().ok()?;
    if stored as usize != bytes.len() {
        return None;
    }
    writer.DetachStream().ok()?;
    stream.cast().ok()
}

fn render_pdf(bytes: &[u8]) -> Option<Content> {
    let input = memory_stream(bytes)?;
    let document = PdfDocument::LoadFromStreamAsync(&input).ok()?.join().ok()?;
    let pages = document.PageCount().ok()?;
    let page = document.GetPage(0).ok()?;
    let size = page.Size().ok()?;
    let longest = size.Width.max(size.Height);
    if !size.Width.is_finite()
        || !size.Height.is_finite()
        || size.Width <= 0.0
        || size.Height <= 0.0
    {
        return None;
    }
    let options = PdfPageRenderOptions::new().ok()?;
    let scale = |length: f32| {
        ((length / longest) * IMAGE_SIDE as f32)
            .round()
            .clamp(1.0, IMAGE_SIDE as f32) as u32
    };
    options.SetDestinationWidth(scale(size.Width)).ok()?;
    options.SetDestinationHeight(scale(size.Height)).ok()?;
    let output = InMemoryRandomAccessStream::new().ok()?;
    page.RenderWithOptionsToStreamAsync(&output, &options)
        .ok()?
        .join()
        .ok()?;
    let _ = page.Close();
    output.Seek(0).ok()?;
    let stream = output.cast().ok()?;
    Artwork::decode_stream(&stream, IMAGE_SIDE, None).map(|image| Content::Pdf { image, pages })
}

fn text_content(bytes: &[u8], cut: bool) -> Option<String> {
    let value = match bytes {
        [0xff, 0xfe, rest @ ..] | [0xfe, 0xff, rest @ ..] => {
            let little_endian = bytes[0] == 0xff;
            let (units, tail) = rest.as_chunks::<2>();
            if !cut && !tail.is_empty() {
                return None;
            }
            let mut samples = units
                .iter()
                .map(|unit| {
                    if little_endian {
                        u16::from_le_bytes(*unit)
                    } else {
                        u16::from_be_bytes(*unit)
                    }
                })
                .collect::<Vec<_>>();
            if cut
                && samples
                    .last()
                    .is_some_and(|last| (0xd800..=0xdbff).contains(last))
            {
                samples.pop();
            }
            String::from_utf16(&samples).ok()?
        }
        _ => {
            let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
            match std::str::from_utf8(bytes) {
                Ok(text) => text.to_owned(),
                Err(error) if cut && error.error_len().is_none() => {
                    std::str::from_utf8(&bytes[..error.valid_up_to()])
                        .ok()?
                        .to_owned()
                }
                _ => return None,
            }
        }
    };
    if value
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\r' | '\n' | '\t'))
    {
        return None;
    }
    // Native multiline edits use CRLF regardless of the source file's line endings.
    Some(
        value
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', "\r\n"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_supported_text_and_rejects_binary_controls() {
        assert_eq!(
            text_content(b"# Heading\nText", false).as_deref(),
            Some("# Heading\r\nText")
        );
        assert_eq!(
            text_content(&[0xfe, 0xff, 0, b'A', 0, b'\r'], false).as_deref(),
            Some("A\r\n")
        );
        assert_eq!(
            text_content(b"Ascii\xe2\x80", true).as_deref(),
            Some("Ascii")
        );
        assert!(text_content(b"Binary\0data", false).is_none());
        assert!(text_content(b"Broken\xfffile", false).is_none());
    }
}
