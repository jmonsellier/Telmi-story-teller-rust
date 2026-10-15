//! Dessin logiciel : toile 640×480, images PNG et cache borné en octets.

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::sdl::{HEIGHT, WIDTH};

/// Taille maximale d'une image décodée avant mise à l'échelle (4096×4096 RGBA).
const MAX_DECODED_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub u8, pub u8, pub u8);

impl Color {
    pub const BLACK: Color = Color(0, 0, 0);
    pub const WHITE: Color = Color(255, 255, 255);
    pub const WHITE_60: Color = Color(189, 186, 193);
    pub const PURPLE: Color = Color(37, 16, 58);
    pub const ORANGE: Color = Color(255, 181, 0);
    pub const YELLOW: Color = Color(255, 186, 0);
    pub const RED: Color = Color(238, 45, 0);

    fn pixel(self) -> u32 {
        (self.0 as u32) << 16 | (self.1 as u32) << 8 | self.2 as u32
    }
}

/// Image décodée, en ARGB 8 bits par canal (alpha non prémultiplié).
pub struct Image {
    pub width: usize,
    pub height: usize,
    pixels: Vec<u32>,
    opaque: bool,
}

impl Image {
    pub fn bytes(&self) -> usize {
        self.pixels.len() * 4
    }

    /// Décode un PNG, en refusant les images démesurées.
    pub fn load_png(path: &Path) -> Option<Image> {
        let file = File::open(path).ok()?;
        let mut decoder = png::Decoder::new_with_limits(
            BufReader::new(file),
            png::Limits { bytes: MAX_DECODED_BYTES },
        );
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().ok()?;
        let mut buffer = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buffer).ok()?;
        let (width, height) = (info.width as usize, info.height as usize);
        let data = &buffer[..info.buffer_size()];
        let channels = info.color_type.samples();
        if width == 0 || height == 0 || data.len() < width * height * channels {
            return None;
        }
        let pixels: Vec<u32> = data
            .chunks_exact(channels)
            .take(width * height)
            .map(|p| {
                let (r, g, b, a) = match channels {
                    1 => (p[0], p[0], p[0], 255),
                    2 => (p[0], p[0], p[0], p[1]),
                    3 => (p[0], p[1], p[2], 255),
                    _ => (p[0], p[1], p[2], p[3]),
                };
                (a as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | b as u32
            })
            .collect();
        let opaque = pixels.iter().all(|p| p >> 24 == 255);
        Some(Image { width, height, pixels, opaque })
    }

    /// Redimensionne à la largeur voulue en gardant les proportions,
    /// comme `rotozoomSurface` de Telmi (moyenne par zone pour réduire,
    /// bilinéaire pour agrandir).
    pub fn scaled_to_width(self, width: usize) -> Image {
        if width == self.width || width == 0 {
            return self;
        }
        let height = ((self.height * width) as f64 / self.width as f64).round().max(1.0) as usize;
        let pixels = if width < self.width {
            self.downscale(width, height)
        } else {
            self.upscale(width, height)
        };
        Image { width, height, pixels, opaque: self.opaque }
    }

    fn downscale(&self, width: usize, height: usize) -> Vec<u32> {
        let mut out = Vec::with_capacity(width * height);
        for y in 0..height {
            let y0 = y * self.height / height;
            let y1 = ((y + 1) * self.height / height).max(y0 + 1);
            for x in 0..width {
                let x0 = x * self.width / width;
                let x1 = ((x + 1) * self.width / width).max(x0 + 1);
                let mut sum = [0u32; 4];
                for sy in y0..y1 {
                    for &p in &self.pixels[sy * self.width + x0..sy * self.width + x1] {
                        for (c, s) in sum.iter_mut().enumerate() {
                            *s += (p >> (24 - 8 * c)) & 0xFF;
                        }
                    }
                }
                let n = ((y1 - y0) * (x1 - x0)) as u32;
                out.push(sum.iter().fold(0, |acc, s| acc << 8 | (s / n)));
            }
        }
        out
    }

    fn upscale(&self, width: usize, height: usize) -> Vec<u32> {
        let mut out = Vec::with_capacity(width * height);
        let sx = self.width as f32 / width as f32;
        let sy = self.height as f32 / height as f32;
        for y in 0..height {
            let fy = ((y as f32 + 0.5) * sy - 0.5).max(0.0);
            let y0 = (fy as usize).min(self.height - 1);
            let y1 = (y0 + 1).min(self.height - 1);
            let ty = fy - y0 as f32;
            for x in 0..width {
                let fx = ((x as f32 + 0.5) * sx - 0.5).max(0.0);
                let x0 = (fx as usize).min(self.width - 1);
                let x1 = (x0 + 1).min(self.width - 1);
                let tx = fx - x0 as f32;
                let [a, b, c, d] = [
                    self.pixels[y0 * self.width + x0],
                    self.pixels[y0 * self.width + x1],
                    self.pixels[y1 * self.width + x0],
                    self.pixels[y1 * self.width + x1],
                ];
                let mut pixel = 0u32;
                for shift in [24, 16, 8, 0] {
                    let ch = |p: u32| ((p >> shift) & 0xFF) as f32;
                    let top = ch(a) + (ch(b) - ch(a)) * tx;
                    let bottom = ch(c) + (ch(d) - ch(c)) * tx;
                    pixel = pixel << 8 | (top + (bottom - top) * ty).round() as u32;
                }
                out.push(pixel);
            }
        }
        out
    }
}

/// Surface de dessin 640×480 (0x00RRGGBB).
pub struct Canvas {
    pub pixels: Vec<u32>,
}

impl Default for Canvas {
    fn default() -> Self {
        Canvas { pixels: vec![0; WIDTH * HEIGHT] }
    }
}

impl Canvas {
    pub fn clear(&mut self) {
        self.pixels.fill(0);
    }

    pub fn copy_from(&mut self, other: &Canvas) {
        self.pixels.copy_from_slice(&other.pixels);
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: Color) {
        let (x0, y0) = (x.max(0) as usize, y.max(0) as usize);
        let x1 = (x.saturating_add(w)).clamp(0, WIDTH as i32) as usize;
        let y1 = (y.saturating_add(h)).clamp(0, HEIGHT as i32) as usize;
        if x0 >= x1 {
            return;
        }
        for row in y0..y1 {
            self.pixels[row * WIDTH + x0..row * WIDTH + x1].fill(color.pixel());
        }
    }

    /// Copie une image en (x, y) avec mélange alpha, découpée aux bords.
    pub fn blit(&mut self, image: &Image, x: i32, y: i32) {
        for iy in 0..image.height as i32 {
            let dy = y + iy;
            if !(0..HEIGHT as i32).contains(&dy) {
                continue;
            }
            let ix0 = (-x).max(0);
            let ix1 = (WIDTH as i32 - x).min(image.width as i32);
            if ix0 >= ix1 {
                return;
            }
            let src = &image.pixels[iy as usize * image.width..][ix0 as usize..ix1 as usize];
            let start = dy as usize * WIDTH + (x + ix0) as usize;
            let dst = &mut self.pixels[start..start + src.len()];
            if image.opaque {
                for (d, s) in dst.iter_mut().zip(src) {
                    *d = s & 0x00FF_FFFF;
                }
            } else {
                for (d, &s) in dst.iter_mut().zip(src) {
                    *d = blend(*d, s & 0x00FF_FFFF, s >> 24);
                }
            }
        }
    }

    /// Mélange une couleur avec une couverture 0‑255 (rendu du texte).
    pub fn blend_pixel(&mut self, x: i32, y: i32, color: Color, coverage: u8) {
        if coverage == 0 || !(0..WIDTH as i32).contains(&x) || !(0..HEIGHT as i32).contains(&y) {
            return;
        }
        let d = &mut self.pixels[y as usize * WIDTH + x as usize];
        *d = blend(*d, color.pixel(), coverage as u32);
    }
}

fn blend(dst: u32, src: u32, alpha: u32) -> u32 {
    match alpha {
        0 => dst,
        255 => src,
        _ => {
            let inv = 255 - alpha;
            let rb = ((src & 0xFF00FF) * alpha + (dst & 0xFF00FF) * inv) >> 8 & 0xFF00FF;
            let g = ((src & 0x00FF00) * alpha + (dst & 0x00FF00) * inv) >> 8 & 0x00FF00;
            rb | g
        }
    }
}

/// Cache LRU des images mises à l'échelle, plafonné en octets.
pub struct ImageCache {
    budget: usize,
    used: usize,
    tick: u64,
    entries: HashMap<(PathBuf, usize), (Rc<Image>, u64)>,
    /// Fichiers absents ou illisibles, pour ne pas les relire à chaque image.
    failures: HashMap<PathBuf, u64>,
}

impl ImageCache {
    pub fn new(budget: usize) -> Self {
        ImageCache { budget, used: 0, tick: 0, entries: HashMap::new(), failures: HashMap::new() }
    }

    /// Image à la largeur demandée (0 = taille d'origine).
    pub fn get(&mut self, path: &Path, width: usize) -> Option<Rc<Image>> {
        self.tick += 1;
        let key = (path.to_path_buf(), width);
        if let Some((image, used)) = self.entries.get_mut(&key) {
            *used = self.tick;
            return Some(image.clone());
        }
        if self.failures.contains_key(path) {
            return None;
        }
        let Some(image) = Image::load_png(path) else {
            if self.failures.len() > 256 {
                self.failures.clear();
            }
            self.failures.insert(path.to_path_buf(), self.tick);
            return None;
        };
        let image = Rc::new(image.scaled_to_width(width));
        self.used += image.bytes();
        self.entries.insert(key, (image.clone(), self.tick));
        self.evict();
        Some(image)
    }

    /// Libère les images les moins récemment utilisées au-delà du budget.
    fn evict(&mut self) {
        while self.used > self.budget && self.entries.len() > 1 {
            let Some(oldest) = self.entries.iter().min_by_key(|(_, (_, used))| *used).map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some((image, _)) = self.entries.remove(&oldest) {
                self.used -= image.bytes();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: usize, height: usize, pixel: u32) -> Image {
        Image { width, height, pixels: vec![pixel; width * height], opaque: pixel >> 24 == 255 }
    }

    #[test]
    fn scales_keeping_ratio() {
        let small = image(640, 480, 0xFF10_2030).scaled_to_width(171);
        assert_eq!((small.width, small.height), (171, 128));
        assert!(small.pixels.iter().all(|&p| p == 0xFF10_2030));
        let big = image(320, 240, 0xFF00_00FF).scaled_to_width(640);
        assert_eq!((big.width, big.height), (640, 480));
        assert!(big.pixels.iter().all(|&p| p == 0xFF00_00FF));
    }

    #[test]
    fn blits_with_clipping_and_alpha() {
        let mut canvas = Canvas::default();
        canvas.blit(&image(10, 10, 0xFFFF_FFFF), -5, 475);
        assert_eq!(canvas.pixels[475 * WIDTH], 0xFF_FFFF);
        assert_eq!(canvas.pixels[475 * WIDTH + 5], 0);
        canvas.blit(&image(1, 1, 0x80FF_0000), 100, 100);
        let p = canvas.pixels[100 * WIDTH + 100];
        assert!((0x7F..=0x80).contains(&(p >> 16)), "{p:x}");
    }

    #[test]
    fn cache_respects_budget() {
        let dir = std::env::temp_dir().join(format!("telmi-gfx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.png");
        let file = File::create(&path).unwrap();
        let mut encoder = png::Encoder::new(file, 100, 100);
        encoder.set_color(png::ColorType::Rgb);
        encoder.write_header().unwrap().write_image_data(&vec![200; 100 * 100 * 3]).unwrap();

        let mut cache = ImageCache::new(50_000);
        assert!(cache.get(&path, 0).is_some());
        assert!(cache.get(&path, 50).is_some());
        assert!(cache.used <= 50_000 || cache.entries.len() == 1);
        assert!(cache.get(&dir.join("absent.png"), 0).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
