//! Bounded WinRT artwork decoding and bitmap presentation.

use std::mem::size_of;
use windows::{
    Graphics::Imaging::{
        BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, BitmapTransform, ColorManagementMode,
        ExifOrientationMode,
    },
    Storage::Streams::IRandomAccessStreamReference,
};
use windows_sys::Win32::{Foundation::RECT, Graphics::Gdi::*};

const MAX_STREAM: u64 = 16 * 1024 * 1024;
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
        let stream = reference.OpenReadAsync().ok()?.join().ok()?;
        let size = stream.Size().ok()?;
        if size == 0 || size > MAX_STREAM {
            return None;
        }
        let decoder = BitmapDecoder::CreateAsync(&stream).ok()?.join().ok()?;
        let (width, height) = fit(decoder.PixelWidth().ok()?, decoder.PixelHeight().ok()?)?;
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

fn fit(width: u32, height: u32) -> Option<(u32, u32)> {
    if width == 0
        || height == 0
        || width.max(height) > 8192
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return None;
    }
    let longest = width.max(height);
    if longest <= SIDE {
        Some((width, height))
    } else {
        let scale = |dimension| {
            ((u64::from(dimension) * u64::from(SIDE) / u64::from(longest)) as u32).max(1)
        };
        Some((scale(width), scale(height)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_cover_dimensions_and_preserves_aspect_ratio() {
        assert_eq!(fit(1200, 600), Some((64, 32)));
        assert_eq!(fit(0, 1), None);
        assert_eq!(fit(10000, 10000), None);
        assert_eq!(fit(8, 16), Some((8, 16)));
    }
}
