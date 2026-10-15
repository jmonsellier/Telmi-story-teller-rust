//! Rendu de texte avec les polices Exo 2 de Telmi, cache de glyphes borné.

use std::collections::HashMap;
use std::path::Path;

use fontdue::{Font, FontSettings, Metrics};

use crate::gfx::{Canvas, Color};

const MAX_CACHED_GLYPHS: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Weight {
    Regular,
    Bold,
}

/// Police, taille et couleur d'un texte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub weight: Weight,
    pub size: u16,
    pub color: Color,
}

impl Style {
    pub const fn new(weight: Weight, size: u16, color: Color) -> Style {
        Style { weight, size, color }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
    Center,
}

pub struct Text {
    regular: Option<Font>,
    bold: Option<Font>,
    glyphs: HashMap<(Weight, u16, char), (Metrics, Vec<u8>)>,
}

impl Text {
    /// Charge `Exo2-Regular.ttf` et `Exo2-Bold.ttf` ; une police absente
    /// désactive simplement le texte au lieu d'arrêter l'application.
    pub fn load(res: &Path) -> Text {
        let font = |name: &str| {
            let bytes = std::fs::read(res.join(name)).ok()?;
            Font::from_bytes(bytes, FontSettings::default()).ok()
        };
        Text {
            regular: font("Exo2-Regular.ttf"),
            bold: font("Exo2-Bold.ttf"),
            glyphs: HashMap::new(),
        }
    }

    fn font(&self, weight: Weight) -> Option<&Font> {
        match weight {
            Weight::Regular => self.regular.as_ref(),
            Weight::Bold => self.bold.as_ref().or(self.regular.as_ref()),
        }
    }

    pub fn width(&self, text: &str, weight: Weight, size: u16) -> i32 {
        let Some(font) = self.font(weight) else { return 0 };
        text.chars().map(|c| font.metrics(c, size as f32).advance_width).sum::<f32>().round() as i32
    }

    /// Écrit `text` dont le haut de la ligne est en `y`, comme `TTF_RenderUTF8`.
    pub fn draw(&mut self, canvas: &mut Canvas, text: &str, style: Style, x: i32, y: i32, align: Align) {
        let Style { weight, size, color } = style;
        let Some(ascent) = self
            .font(weight)
            .and_then(|f| f.horizontal_line_metrics(size as f32))
            .map(|m| m.ascent)
        else {
            return;
        };
        let width = self.width(text, weight, size);
        let mut pen = match align {
            Align::Left => x as f32,
            Align::Right => (x - width) as f32,
            Align::Center => (x - width / 2) as f32,
        };
        let baseline = y as f32 + ascent;
        for c in text.chars() {
            if self.glyphs.len() >= MAX_CACHED_GLYPHS {
                self.glyphs.clear();
            }
            let key = (weight, size, c);
            if !self.glyphs.contains_key(&key) {
                let Some(font) = self.font(weight) else { return };
                self.glyphs.insert(key, font.rasterize(c, size as f32));
            }
            let (metrics, bitmap) = &self.glyphs[&key];
            let gx = (pen + metrics.xmin as f32).round() as i32;
            let gy = (baseline - (metrics.ymin + metrics.height as i32) as f32).round() as i32;
            for row in 0..metrics.height {
                for col in 0..metrics.width {
                    let coverage = bitmap[row * metrics.width + col];
                    canvas.blend_pixel(gx + col as i32, gy + row as i32, color, coverage);
                }
            }
            pen += metrics.advance_width;
        }
    }
}
