//! Bounded WinRT artwork decoding and bitmap presentation.

use std::mem::size_of;
use windows::{
    Graphics::Imaging::{
        BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, BitmapTransform, ColorManagementMode,
        ExifOrientationMode,
    },
    Storage::Streams::{IRandomAccessStream, IRandomAccessStreamReference},
    core::{GUID, Interface},
};
use windows_sys::Win32::{Foundation::RECT, Graphics::Gdi::*};

pub const MAX_STREAM: u64 = 16 * 1024 * 1024;
const MAX_PIXELS: u64 = 16 * 1024 * 1024;
const SIDE: u32 = 64;

#[derive(Debug, PartialEq, Eq)]
pub struct Artwork {
    width: u32,
    height: u32,
    pixels: Box<[u8]>,
}

impl Artwork {
    pub fn decode(reference: &IRandomAccessStreamReference) -> Option<Self> {
        let stream = reference.OpenReadAsync().ok()?.join().ok()?.cast().ok()?;
        Self::decode_stream(&stream, SIDE, None)
    }

    pub fn decode_stream(
        stream: &IRandomAccessStream,
        max_side: u32,
        codec: Option<GUID>,
    ) -> Option<Self> {
        let size = stream.Size().ok()?;
        if size == 0 || size > MAX_STREAM {
            return None;
        }
        let decoder = match codec {
            Some(codec) => BitmapDecoder::CreateWithIdAsync(codec, stream),
            None => BitmapDecoder::CreateAsync(stream),
        }
        .ok()?
        .join()
        .ok()?;
        let (width, height) = fit(
            decoder.PixelWidth().ok()?,
            decoder.PixelHeight().ok()?,
            max_side,
        )?;
        let transform = BitmapTransform::new().ok()?;
        transform.SetScaledWidth(width).ok()?;
        transform.SetScaledHeight(height).ok()?;
        let pixels = decoder
            .GetPixelDataTransformedAsync(
                BitmapPixelFormat::Bgra8,
                BitmapAlphaMode::Premultiplied,
                &transform,
                ExifOrientationMode::IgnoreExifOrientation,
                ColorManagementMode::DoNotColorManage,
            )
            .ok()?
            .join()
            .ok()?
            .DetachPixelData()
            .ok()?;
        if pixels.len() != (width * height * 4) as usize {
            return None;
        }
        Some(Self {
            width,
            height,
            pixels: pixels.to_vec().into_boxed_slice(),
        })
    }

    /// Wrap top-down BGRA rows, premultiplying alpha if the data is straight.
    pub fn from_bgra(width: u32, height: u32, mut pixels: Vec<u8>) -> Option<Self> {
        if width == 0 || height == 0 || pixels.len() != (width * height * 4) as usize {
            return None;
        }
        // Premultiplied colour channels can never exceed alpha.
        let straight = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[..3].iter().any(|&channel| channel > pixel[3]));
        if straight {
            for pixel in pixels.as_chunks_mut::<4>().0 {
                let alpha = u16::from(pixel[3]);
                for channel in &mut pixel[..3] {
                    *channel = (u16::from(*channel) * alpha / 255) as u8;
                }
            }
        }
        Some(Self {
            width,
            height,
            pixels: pixels.into_boxed_slice(),
        })
    }

    pub fn draw(&self, dc: HDC, area: RECT, background: u32) {
        let width = (area.right - area.left).max(0);
        let height = (area.bottom - area.top).max(0);
        if width == 0 || height == 0 {
            return;
        }
        let image_width = self.width as i32;
        let image_height = self.height as i32;
        let width_first = image_width * height >= image_height * width;
        let (w, h) = if width_first {
            (width, (image_height * width / image_width).max(1))
        } else {
            ((image_width * height / image_height).max(1), height)
        };
        let source = self
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| {
                let remaining = 255 - u32::from(pixel[3]);
                let blend = |index: usize, shift: u32| {
                    (u32::from(pixel[index]) + ((background >> shift) & 255) * remaining / 255)
                        .min(255)
                };
                blend(0, 16) | (blend(1, 8) << 8) | (blend(2, 0) << 16)
            })
            .collect::<Vec<u32>>();
        let bitmap = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: image_width,
                biHeight: -image_height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        unsafe {
            StretchDIBits(
                dc,
                area.left + (width - w) / 2,
                area.top + (height - h) / 2,
                w,
                h,
                0,
                0,
                image_width,
                image_height,
                source.as_ptr().cast(),
                &bitmap,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
        }
    }
}

fn fit(width: u32, height: u32, max_side: u32) -> Option<(u32, u32)> {
    if width == 0
        || height == 0
        || width.max(height) > 8192
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return None;
    }
    let longest = width.max(height);
    if longest <= max_side {
        Some((width, height))
    } else {
        let scale = |dimension| {
            ((u64::from(dimension) * u64::from(max_side) / u64::from(longest)) as u32).max(1)
        };
        Some((scale(width), scale(height)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_cover_dimensions_and_preserves_aspect_ratio() {
        assert_eq!(fit(1200, 600, SIDE), Some((64, 32)));
        assert_eq!(fit(0, 1, SIDE), None);
        assert_eq!(fit(10000, 10000, SIDE), None);
        assert_eq!(fit(8, 16, SIDE), Some((8, 16)));
        assert_eq!(fit(2400, 1200, 1024), Some((1024, 512)));
    }

    #[test]
    fn premultiplies_only_straight_alpha() {
        let straight = Artwork::from_bgra(1, 1, vec![200, 100, 50, 128]).unwrap();
        assert_eq!(&*straight.pixels, &[100, 50, 25, 128]);
        let premultiplied = Artwork::from_bgra(1, 1, vec![100, 50, 25, 128]).unwrap();
        assert_eq!(&*premultiplied.pixels, &[100, 50, 25, 128]);
        assert!(Artwork::from_bgra(2, 1, vec![0; 4]).is_none());
    }
}
