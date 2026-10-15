//! Which texture a [`TexRef`] samples: an embedded bitmap or glyph sheet, or a
//! texture of the shared UI dictionary (`resources/ui/shared_latin.r9`) named by
//! the movie.

use crate::draw::TexRef;
use hashbrown::HashMap;
use mcla_formats::rage::swf::{Character, Movie, Sheet};
use mcla_formats::rage::texture::Texture;

pub fn resolve<'a>(movie: &'a Movie, t: TexRef, shared: &'a HashMap<String, Texture>) -> Option<&'a Texture> {
    match t {
        TexRef::Bitmap(id) => match movie.char(id) {
            Some(Character::Bitmap(b)) => match (&b.texture, &b.external) {
                (Some(t), _) => Some(t),
                (None, Some(name)) => shared.get(name),
                _ => None,
            },
            _ => None,
        },
        TexRef::Glyph { font, page, sheet } => match movie.char(font) {
            Some(Character::Font(f)) => match f.pages.get(page as usize).and_then(|p| p.sheets.get(sheet as usize)) {
                Some(Sheet::Embedded(t)) => Some(t),
                Some(Sheet::Shared(n)) => shared.get(n),
                _ => None,
            },
            _ => None,
        },
        TexRef::None | TexRef::Swap(_) => None,
    }
}
