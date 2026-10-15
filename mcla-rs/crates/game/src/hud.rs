//! In-game HUD drawn with the original UI art pulled out of the game's UI scenes
//! (`resources/ui/hud`, `resources/ui/hudmap`): tachometer dial, gauge frames,
//! nitro ring, bitmap speed digits, minimap frame and chevrons.

use crate::assets::load_resource_file;
use egui::{Color32, Mesh, Pos2, Rect, TextureHandle, Vec2};
use hashbrown::HashMap;
use mcla_formats::rage::ui::ui_textures;
use std::path::Path;

/// Glyph rectangles (uv) of a bitmap font sheet.
pub struct BitmapFont {
    pub tex: TextureHandle,
    pub glyphs: HashMap<char, (Rect, f32)>, // uv rect, aspect (w/h)
    pub line_h: f32,
}

pub struct UiArt {
    pub tex: HashMap<String, TextureHandle>,
    pub digits: Option<BitmapFont>,
    /// the game's UI typeface (glyph sheet of the pause scene)
    pub font: Option<TextFont>,
}

#[derive(Clone, Copy)]
pub struct Glyph {
    pub uv: Rect,
    /// size in sheet pixels
    pub w: f32,
    pub h: f32,
    /// pixels above the baseline
    pub above: f32,
}

/// Proportional bitmap font cut out of one of the game's glyph cache sheets.
pub struct TextFont {
    pub tex: TextureHandle,
    pub glyphs: HashMap<char, Glyph>,
    /// height of a capital letter in sheet pixels
    pub cap_h: f32,
}

/// Glyph order of `pause-font100-height500-000` row by row (the sheet has no `<`, `>`, `|`).
const FONT_ROWS: [&str; 10] = [
    "!\"#$%&'()*+,-./01",
    "23456789:;=?@",
    "ABCDEFGHIJK",
    "LMNOPQRSTU",
    "VWXYZ[\\]^_`ab",
    "cdefghijklmnop",
    "qrstuvwxyz{}~\u{a1}",
    "\u{bf}\u{c0}\u{c1}\u{c2}\u{c3}\u{c4}\u{c5}\u{c6}",
    "\u{c7}\u{c8}\u{c9}\u{ca}\u{cb}\u{cc}\u{cd}\u{ce}\u{cf}\u{d1}\u{d2}\u{d3}\u{d4}",
    "\u{d5}\u{d6}\u{d9}\u{da}\u{db}\u{dc}\u{e0}\u{e1}\u{e2}\u{e3}\u{e4}",
];
/// a glyph without descender per row: its bottom edge is the row's baseline
const FONT_BASE: [char; 10] = ['0', '2', 'A', 'L', 'V', 'c', 'r', '\u{c0}', '\u{c8}', '\u{d6}'];

fn build_text_font(tex: TextureHandle, w: u32, h: u32, px: &[u8]) -> Option<TextFont> {
    let (w, h) = (w as usize, h as usize);
    let alpha = |x: usize, y: usize| px[(y * w + x) * 4 + 3];
    let mut bands: Vec<(usize, usize)> = Vec::new();
    let mut y = 0;
    while y < h {
        if (0..w).any(|x| alpha(x, y) > 24) {
            let y0 = y;
            while y < h && (0..w).any(|x| alpha(x, y) > 24) {
                y += 1;
            }
            bands.push((y0, y));
        } else {
            y += 1;
        }
    }
    if bands.len() != FONT_ROWS.len() {
        log::warn!("text font: {} glyph rows found, {} expected", bands.len(), FONT_ROWS.len());
        return None;
    }
    let mut glyphs: HashMap<char, Glyph> = HashMap::new();
    let mut cap_h = 19.0;
    for (ri, &(y0, y1)) in bands.iter().enumerate() {
        // components separated by empty columns
        let col = |x: usize| (y0..y1).any(|yy| alpha(x, yy) > 24);
        let mut comps: Vec<(usize, usize)> = Vec::new();
        let mut x = 0;
        while x < w {
            if col(x) {
                let x0 = x;
                while x < w && col(x) {
                    x += 1;
                }
                comps.push((x0, x));
            } else {
                x += 1;
            }
        }
        let chars: Vec<char> = FONT_ROWS[ri].chars().collect();
        let need: usize = chars.iter().map(|c| if *c == '"' { 2 } else { 1 }).sum();
        if comps.len() != need {
            log::warn!("text font: row {} has {} components, {} expected", ri, comps.len(), need);
            return None;
        }
        let mut ci = 0;
        let mut row: Vec<(char, usize, usize, usize, usize)> = Vec::new();
        for c in chars {
            let (x0, mut x1) = comps[ci];
            ci += 1;
            if c == '"' {
                x1 = comps[ci].1;
                ci += 1;
            }
            // tight vertical extent
            let mut t = y1;
            let mut b = y0;
            for yy in y0..y1 {
                if (x0..x1).any(|xx| alpha(xx, yy) > 24) {
                    t = t.min(yy);
                    b = b.max(yy + 1);
                }
            }
            row.push((c, x0, x1, t, b));
        }
        let base = row.iter().find(|g| g.0 == FONT_BASE[ri]).map(|g| g.4).unwrap_or(y1);
        for (c, x0, x1, t, b) in row {
            if c == 'A' {
                cap_h = (b - t) as f32;
            }
            let uv = Rect::from_min_max(Pos2::new(x0 as f32 / w as f32, t as f32 / h as f32), Pos2::new(x1 as f32 / w as f32, b as f32 / h as f32));
            // the sheet stores every glyph cropped and pushed to the top of its row, so
            // the height above the baseline has to come from the kind of character
            let (gh, cap) = ((b - t) as f32, base as f32 - y0 as f32);
            let above = match c {
                '.' | '_' | ':' => gh,
                ',' | ';' => gh - (gh * 0.3).min(4.0),
                '-' | '+' | '=' | '~' => cap * 0.5 + gh * 0.5,
                'a' | 'c' | 'e' | 'm' | 'n' | 'o' | 'r' | 's' | 'u' | 'v' | 'w' | 'x' | 'z' => gh,
                'g' | 'j' | 'p' | 'q' | 'y' => gh * 0.74,
                _ => base as f32 - t as f32,
            };
            glyphs.insert(c, Glyph { uv, w: (x1 - x0) as f32, h: gh, above });
        }
    }
    log::info!("text font: {} glyphs, cap height {}", glyphs.len(), cap_h);
    Some(TextFont { tex, glyphs, cap_h })
}

impl TextFont {
    fn glyph(&self, c: char) -> Option<&Glyph> {
        self.glyphs.get(&c).or_else(|| self.glyphs.get(&c.to_ascii_uppercase())).or_else(|| match c {
            '<' => self.glyphs.get(&'('),
            '>' => self.glyphs.get(&')'),
            '|' => self.glyphs.get(&'!'),
            _ => None,
        })
    }

    /// Width of `text` at capital height `size` (px).
    pub fn measure(&self, text: &str, size: f32) -> f32 {
        let k = size / self.cap_h;
        let mut w = 0.0;
        for c in text.chars() {
            w += match self.glyph(c) {
                Some(g) => (g.w + 2.0) * k,
                None => size * 0.42,
            };
        }
        w
    }

    /// Draw `text` with capital height `size`; `pos` is anchored according to `align`
    /// (vertically: top = cap top, bottom = baseline). Returns the covered rectangle.
    pub fn draw(&self, painter: &egui::Painter, pos: Pos2, align: egui::Align2, text: &str, size: f32, color: Color32) -> Rect {
        let k = size / self.cap_h;
        let width = self.measure(text, size);
        let x0 = match align.x() {
            egui::Align::Min => pos.x,
            egui::Align::Center => pos.x - width * 0.5,
            egui::Align::Max => pos.x - width,
        };
        let baseline = match align.y() {
            egui::Align::Min => pos.y + size,
            egui::Align::Center => pos.y + size * 0.5,
            egui::Align::Max => pos.y,
        };
        let mut x = x0;
        for c in text.chars() {
            match self.glyph(c) {
                Some(g) => {
                    let r = Rect::from_min_size(Pos2::new(x, baseline - g.above * k), Vec2::new(g.w * k, g.h * k));
                    painter.image(self.tex.id(), r, g.uv, color);
                    x += (g.w + 2.0) * k;
                }
                None => x += size * 0.42,
            }
        }
        Rect::from_min_max(Pos2::new(x0, baseline - size), Pos2::new(x0 + width, baseline + size * 0.25))
    }
}

impl UiArt {
    pub fn load(ctx: &egui::Context, data_dir: &str) -> UiArt {
        let mut tex = HashMap::new();
        let mut raw: HashMap<String, (u32, u32, Vec<u8>)> = HashMap::new();
        for rel in ["resources/ui/hud/0x1f0e1cda.r27", "resources/ui/hudmap/0x7064220c.r27", "resources/ui/pause/0xaed9b6c9.r27"] {
            let p = Path::new(data_dir).join(rel);
            let Ok(res) = load_resource_file(&p) else {
                log::warn!("ui art: cannot load {}", p.display());
                continue;
            };
            for t in ui_textures(&res) {
                let name = t.name.trim_end_matches(".dds").to_string();
                let rgba = t.to_rgba8();
                let img = egui::ColorImage::from_rgba_unmultiplied([t.width as usize, t.height as usize], &rgba);
                let h = ctx.load_texture(&name, img, egui::TextureOptions::LINEAR);
                raw.insert(name.clone(), (t.width, t.height, rgba));
                tex.insert(name, h);
            }
        }
        // the map's icons: the textures of the icon meshes (garage, service station, racer ...)
        if let Ok(res) = load_resource_file(&Path::new(data_dir).join("resources/ui/meshtextures.r9")) {
            use mcla_formats::rage::{texture::Texture, Dictionary, Rd};
            let rd = Rd::new(&res);
            if let Ok(d) = Dictionary::parse(&rd, 0) {
                for o in d.values.iter().flatten() {
                    let Ok(t) = Texture::parse(&rd, *o) else { continue };
                    let name = t.name.rsplit('/').next().unwrap_or("").trim_end_matches(".dds").to_ascii_lowercase();
                    if !name.starts_with("icon_") || tex.contains_key(&name) {
                        continue;
                    }
                    let img = egui::ColorImage::from_rgba_unmultiplied([t.width as usize, t.height as usize], &t.to_rgba8());
                    tex.insert(name.clone(), ctx.load_texture(&name, img, egui::TextureOptions::LINEAR));
                }
            }
        }
        log::info!("ui art: {} textures", tex.len());
        // (MCLA_ART_DUMP=<dir>: the icon textures as PNG files, for looking at them)
        if let Ok(dir) = std::env::var("MCLA_ART_DUMP") {
            let mut names: Vec<&String> = tex.keys().filter(|n| n.starts_with("icon_")).collect();
            names.sort();
            log::info!("ui art icons: {:?}", names);
            if let Ok(res) = load_resource_file(&Path::new(data_dir).join("resources/ui/meshtextures.r9")) {
                use mcla_formats::rage::{texture::Texture, Dictionary, Rd};
                let rd = Rd::new(&res);
                if let Ok(d) = Dictionary::parse(&rd, 0) {
                    for o in d.values.iter().flatten() {
                        let Ok(t) = Texture::parse(&rd, *o) else { continue };
                        let name = t.name.rsplit('/').next().unwrap_or("").trim_end_matches(".dds").to_ascii_lowercase();
                        let mut raw = Vec::with_capacity(8 + (t.width * t.height * 4) as usize);
                        raw.extend_from_slice(&t.width.to_le_bytes());
                        raw.extend_from_slice(&t.height.to_le_bytes());
                        raw.extend_from_slice(&t.to_rgba8());
                        let _ = std::fs::write(Path::new(&dir).join(format!("{name}.rgba")), raw);
                    }
                }
            }
        }
        let digits = raw.get("hud-font042-height1920-000").and_then(|(w, h, px)| {
            let handle = tex.get("hud-font042-height1920-000")?.clone();
            let _ = px;
            // glyph cells of the 256x256 sheet (slanted digits in 48 px rows)
            const CELLS: [(char, u32, u32, u32, u32); 14] = [
                ('\'', 0, 0, 22, 48),
                ('0', 23, 0, 93, 48),
                ('1', 94, 0, 129, 48),
                ('2', 130, 0, 202, 48),
                ('3', 0, 49, 69, 96),
                ('4', 70, 49, 125, 96),
                ('5', 126, 49, 197, 96),
                ('6', 0, 97, 68, 144),
                ('7', 69, 97, 116, 144),
                ('8', 117, 97, 186, 144),
                ('9', 0, 145, 69, 192),
                ('C', 70, 145, 142, 192),
                ('N', 143, 145, 216, 192),
                ('R', 0, 193, 71, 240),
            ];
            let mut glyphs = HashMap::new();
            for (c, x0, y0, x1, y1) in CELLS {
                let uv = Rect::from_min_max(Pos2::new(x0 as f32 / *w as f32, y0 as f32 / *h as f32), Pos2::new(x1 as f32 / *w as f32, y1 as f32 / *h as f32));
                glyphs.insert(c, (uv, (x1 - x0) as f32 / (y1 - y0) as f32));
            }
            Some(BitmapFont { tex: handle, glyphs, line_h: 48.0 })
        });
        let font = raw.get("pause-font100-height500-000").and_then(|(w, h, px)| {
            let handle = tex.get("pause-font100-height500-000")?.clone();
            build_text_font(handle, *w, *h, px)
        });
        UiArt { tex, digits, font }
    }

    pub fn get(&self, name: &str) -> Option<&TextureHandle> {
        self.tex.get(name)
    }
}

/// Split a glyph sheet into rows and glyph columns using the alpha channel and
/// assign the characters of `order` left to right, top to bottom.
#[allow(dead_code)]
fn segment_font(tex: TextureHandle, w: u32, h: u32, px: &[u8], order: &str) -> Option<BitmapFont> {
    let (w, h) = (w as usize, h as usize);
    let alpha = |x: usize, y: usize| px[(y * w + x) * 4 + 3];
    let row_has = |y: usize| (0..w).any(|x| alpha(x, y) > 40);
    // row bands
    let mut bands = Vec::new();
    let mut y = 0;
    while y < h {
        if row_has(y) {
            let y0 = y;
            while y < h && row_has(y) {
                y += 1;
            }
            if y - y0 >= 8 {
                bands.push((y0, y));
            }
        } else {
            y += 1;
        }
    }
    let mut rects = Vec::new();
    for (y0, y1) in &bands {
        let col_has = |x: usize| (*y0..*y1).any(|yy| alpha(x, yy) > 40);
        let mut x = 0;
        while x < w {
            if col_has(x) {
                let x0 = x;
                while x < w && col_has(x) {
                    x += 1;
                }
                if x - x0 >= 3 {
                    rects.push((x0, *y0, x, *y1));
                }
            } else {
                x += 1;
            }
        }
    }
    let chars: Vec<char> = order.chars().collect();
    if rects.len() < chars.len() {
        log::warn!("font segmentation: {} glyphs found, {} expected", rects.len(), chars.len());
        return None;
    }
    let mut glyphs = HashMap::new();
    let mut line_h = 0.0f32;
    for (i, c) in chars.iter().enumerate() {
        let (x0, y0, x1, y1) = rects[i];
        let uv = Rect::from_min_max(Pos2::new(x0 as f32 / w as f32, y0 as f32 / h as f32), Pos2::new(x1 as f32 / w as f32, y1 as f32 / h as f32));
        let aspect = (x1 - x0) as f32 / (y1 - y0) as f32;
        line_h = line_h.max((y1 - y0) as f32);
        glyphs.insert(*c, (uv, aspect));
    }
    Some(BitmapFont { tex, glyphs, line_h })
}

impl BitmapFont {
    /// Draw `text` with glyph height `size`, right-aligned at `right` (baseline top at `top`).
    /// Returns the total width.
    pub fn draw_right(&self, painter: &egui::Painter, text: &str, right: f32, top: f32, size: f32, tint: Color32) -> f32 {
        let gap = size * 0.06;
        let mut widths = Vec::new();
        for c in text.chars() {
            widths.push(self.glyphs.get(&c).map(|g| g.1 * size).unwrap_or(size * 0.4));
        }
        let total: f32 = widths.iter().sum::<f32>() + gap * widths.len().saturating_sub(1) as f32;
        let mut x = right - total;
        for (c, wdt) in text.chars().zip(widths) {
            if let Some((uv, _)) = self.glyphs.get(&c) {
                let rect = Rect::from_min_size(Pos2::new(x, top), Vec2::new(wdt, size));
                painter.image(self.tex.id(), rect, *uv, tint);
            }
            x += wdt + gap;
        }
        total
    }
}

/// Draw a texture centred at `c` with `size`, rotated by `angle` (radians, clockwise on screen).
pub fn image_rot(painter: &egui::Painter, tex: &TextureHandle, c: Pos2, size: Vec2, angle: f32, tint: Color32) {
    let rect = Rect::from_center_size(c, size);
    let mut mesh = Mesh::with_texture(tex.id());
    mesh.add_rect_with_uv(rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
    if angle.abs() > 1e-4 {
        mesh.rotate(egui::emath::Rot2::from_angle(angle), c);
    }
    painter.add(egui::Shape::mesh(mesh));
}

pub fn image(painter: &egui::Painter, tex: &TextureHandle, rect: Rect, tint: Color32) {
    painter.image(tex.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
}

/// Thick arc (screen angles in radians, 0 = right, clockwise positive) as a filled band.
pub fn arc(painter: &egui::Painter, c: Pos2, r_in: f32, r_out: f32, a0: f32, a1: f32, color: Color32) {
    if (a1 - a0).abs() < 1e-3 {
        return;
    }
    let steps = ((a1 - a0).abs() / 0.05).ceil().max(2.0) as usize;
    let mut mesh = Mesh::default();
    for i in 0..=steps {
        let a = a0 + (a1 - a0) * i as f32 / steps as f32;
        let (s, co) = a.sin_cos();
        let p_in = Pos2::new(c.x + co * r_in, c.y + s * r_in);
        let p_out = Pos2::new(c.x + co * r_out, c.y + s * r_out);
        mesh.colored_vertex(p_in, color);
        mesh.colored_vertex(p_out, color);
        if i > 0 {
            let b = (i as u32 - 1) * 2;
            mesh.add_triangle(b, b + 1, b + 2);
            mesh.add_triangle(b + 1, b + 3, b + 2);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Dial geometry shared by the tachometer art: 0 at 225 deg (bottom-left), full scale
/// at -45 deg (bottom-right), clockwise on screen.
pub fn dial_angle(frac: f32) -> f32 {
    let start = 135f32.to_radians(); // screen angle of "0" (y down): bottom-left
    start + frac.clamp(0.0, 1.0) * 270f32.to_radians()
}

/// Draw a rounded widget texture stretched over `rect` keeping its corners (`border`
/// = corner size on screen; the texture's corner is the outer 30 % of its size).
pub fn nine_slice(painter: &egui::Painter, tex: &TextureHandle, rect: Rect, border: f32, tint: Color32) {
    let b = border.min(rect.width() * 0.5).min(rect.height() * 0.5);
    let u = 0.30;
    let xs = [rect.left(), rect.left() + b, rect.right() - b, rect.right()];
    let ys = [rect.top(), rect.top() + b, rect.bottom() - b, rect.bottom()];
    let us = [0.02, u, 1.0 - u, 0.98];
    let mut mesh = Mesh::with_texture(tex.id());
    for i in 0..3 {
        for j in 0..3 {
            let r = Rect::from_min_max(Pos2::new(xs[i], ys[j]), Pos2::new(xs[i + 1], ys[j + 1]));
            if r.width() <= 0.0 || r.height() <= 0.0 {
                continue;
            }
            mesh.add_rect_with_uv(r, Rect::from_min_max(Pos2::new(us[i], us[j]), Pos2::new(us[i + 1], us[j + 1])), tint);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}
