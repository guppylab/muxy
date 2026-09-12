use std::sync::Arc;

pub const MAX_GRAPHICS_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_GRAPHICS_PLACEMENTS: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Graphics {
    pub cell: CellSize,
    pub images: Vec<GraphicImage>,
    pub placements: Vec<GraphicPlacement>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphicImage {
    pub id: u32,
    pub generation: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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
