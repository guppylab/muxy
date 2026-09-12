use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const MAX_GRAPHICS_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_GRAPHICS_PLACEMENTS: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CellSize {
    pub width: u16,
    pub height: u16,
}

impl Default for CellSize {
    fn default() -> Self {
        Self {
            width: 8,
            height: 16,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Graphics {
    pub cell: CellSize,
    pub images: Vec<GraphicImage>,
    pub placements: Vec<GraphicPlacement>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GraphicImage {
    pub id: u32,
    pub generation: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GraphicPlacement {
    pub image: u32,
    pub id: u32,
    pub column: i32,
    pub row: i32,
    pub offset: [u32; 2],
    pub size: [u32; 2],
    pub source: [u32; 4],
    pub z: i32,
}

impl Graphics {
    pub fn validate(&self) -> Result<(), crate::ErrorCode> {
        use crate::ErrorCode::BadRequest;
        if !(1..=4096).contains(&self.cell.width)
            || !(1..=4096).contains(&self.cell.height)
            || self.images.len() > MAX_GRAPHICS_PLACEMENTS
            || self.placements.len() > MAX_GRAPHICS_PLACEMENTS
        {
            return Err(BadRequest);
        }
        let mut images = std::collections::BTreeMap::new();
        let mut total = 0_usize;
        for image in &self.images {
            let bytes = u64::from(image.width)
                .checked_mul(u64::from(image.height))
                .and_then(|pixels| pixels.checked_mul(4))
                .ok_or(BadRequest)?;
            total = total.checked_add(image.rgba.len()).ok_or(BadRequest)?;
            if image.width == 0
                || image.height == 0
                || bytes != image.rgba.len() as u64
                || total > MAX_GRAPHICS_BYTES
                || images.insert(image.id, image).is_some()
            {
                return Err(BadRequest);
            }
        }
        for placement in &self.placements {
            let image = images.get(&placement.image).ok_or(BadRequest)?;
            let [x, y, width, height] = placement.source;
            if width == 0
                || height == 0
                || placement.size.contains(&0)
                || x.checked_add(width).is_none_or(|end| end > image.width)
                || y.checked_add(height).is_none_or(|end| end > image.height)
            {
                return Err(BadRequest);
            }
        }
        Ok(())
    }
}
