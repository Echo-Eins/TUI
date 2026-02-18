//! Screen capture module - captures screen and compresses to JPEG

use image::{imageops::FilterType, DynamicImage, RgbImage};
use scrap::{Capturer, Display};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("No display found")]
    NoDisplay,
    #[error("Failed to initialize capturer: {0}")]
    InitError(String),
    #[error("Capture failed: {0}")]
    CaptureError(String),
    #[error("JPEG compression failed: {0}")]
    CompressionError(String),
    #[error("Invalid capture region")]
    InvalidRegion,
}

pub struct ScreenCapturer {
    capturer: Capturer,
    width: usize,
    height: usize,
    target_width: u32,
    target_height: u32,
    jpeg_quality: u8,
    capture_region: Option<CaptureRegion>,
    last_frame_hash: AtomicU32,
    frame_counter: AtomicU32,
    running: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Copy)]
pub struct CaptureRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl ScreenCapturer {
    pub fn new(
        target_width: u32,
        target_height: u32,
        jpeg_quality: u8,
        capture_region: Option<[u32; 4]>,
    ) -> Result<Self, CaptureError> {
        let display = Display::primary().map_err(|e| CaptureError::InitError(e.to_string()))?;
        let width = display.width();
        let height = display.height();
        let capturer = Capturer::new(display).map_err(|e| CaptureError::InitError(e.to_string()))?;

        let region = capture_region.map(|r| CaptureRegion {
            x: r[0], y: r[1], width: r[2], height: r[3],
        });

        if let Some(ref r) = region {
            if r.x + r.width > width as u32 || r.y + r.height > height as u32 {
                return Err(CaptureError::InvalidRegion);
            }
        }

        // Clamp JPEG quality to valid range 1-100
        let jpeg_quality = jpeg_quality.clamp(1, 100);

        Ok(Self {
            capturer, width, height, target_width, target_height, jpeg_quality,
            capture_region: region,
            last_frame_hash: AtomicU32::new(0),
            frame_counter: AtomicU32::new(0),
            running: Arc::new(AtomicBool::new(true)),
        })
    }

    pub fn get_running_flag(&self) -> Arc<AtomicBool> {
        self.running.clone()
    }

    pub fn capture_frame(&mut self) -> Result<Option<CapturedFrame>, CaptureError> {
        // Capture and immediately copy to owned Vec to avoid borrow issues
        let buffer: Vec<u8> = loop {
            match self.capturer.frame() {
                Ok(frame) => break frame.to_vec(),
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::WouldBlock {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }
                    return Err(CaptureError::CaptureError(e.to_string()));
                }
            }
        };

        let rgb_data = self.bgra_to_rgb(&buffer);
        let hash = compute_hash(&rgb_data);
        let last_hash = self.last_frame_hash.load(Ordering::Relaxed);

        if hash == last_hash && last_hash != 0 {
            return Ok(None);
        }
        self.last_frame_hash.store(hash, Ordering::Relaxed);

        let (src_width, src_height) = if let Some(ref region) = self.capture_region {
            (region.width as usize, region.height as usize)
        } else {
            (self.width, self.height)
        };

        let image = RgbImage::from_raw(src_width as u32, src_height as u32, rgb_data)
            .ok_or_else(|| CaptureError::CaptureError("Failed to create image".into()))?;

        let dynamic = DynamicImage::ImageRgb8(image);
        let resized = dynamic.resize_exact(self.target_width, self.target_height, FilterType::Triangle);

        let jpeg_data = compress_jpeg(&resized, self.jpeg_quality)?;
        let sequence = self.frame_counter.fetch_add(1, Ordering::Relaxed);

        Ok(Some(CapturedFrame {
            sequence,
            jpeg_data,
            width: self.target_width,
            height: self.target_height,
        }))
    }

    fn bgra_to_rgb(&self, bgra: &[u8]) -> Vec<u8> {
        let stride = self.width * 4;
        let (start_x, start_y, crop_width, crop_height) = if let Some(ref region) = self.capture_region {
            (region.x as usize, region.y as usize, region.width as usize, region.height as usize)
        } else {
            (0, 0, self.width, self.height)
        };

        let mut rgb = Vec::with_capacity(crop_width * crop_height * 3);
        for y in start_y..(start_y + crop_height) {
            for x in start_x..(start_x + crop_width) {
                let offset = y * stride + x * 4;
                if offset + 2 < bgra.len() {
                    rgb.push(bgra[offset + 2]); // R
                    rgb.push(bgra[offset + 1]); // G
                    rgb.push(bgra[offset]);     // B
                }
            }
        }
        rgb
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

fn compute_hash(rgb: &[u8]) -> u32 {
    let sample_count = 256;
    let step = rgb.len() / sample_count;
    if step == 0 { return 0; }
    let mut hash: u32 = 0;
    for i in 0..sample_count {
        let idx = i * step;
        if idx < rgb.len() {
            hash = hash.wrapping_mul(31).wrapping_add(rgb[idx] as u32);
        }
    }
    hash
}

fn compress_jpeg(image: &DynamicImage, quality: u8) -> Result<Vec<u8>, CaptureError> {
    use image::codecs::jpeg::JpegEncoder;

    let mut jpeg_data = Vec::new();
    let rgb_image = image.to_rgb8();
    let mut encoder = JpegEncoder::new_with_quality(&mut jpeg_data, quality);
    encoder.encode(
        rgb_image.as_raw(),
        rgb_image.width(),
        rgb_image.height(),
        image::ColorType::Rgb8,
    ).map_err(|e| CaptureError::CompressionError(e.to_string()))?;
    Ok(jpeg_data)
}

#[derive(Debug, Clone)]
pub struct CapturedFrame {
    pub sequence: u32,
    pub jpeg_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl CapturedFrame {
    pub fn size(&self) -> usize {
        self.jpeg_data.len()
    }
}
