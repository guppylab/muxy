use std::hash::{DefaultHasher, Hash, Hasher};

pub use muxy_protocol::{
    Color, Cursor, CursorShape, InputModes, Modes, Modifiers, MouseAction, MouseButton, MouseEvent,
    Row, Run, ScrollDirection, Size, Style, Underline,
};

pub(crate) fn hash_runs(runs: &[Run]) -> u64 {
    let mut hasher = DefaultHasher::new();
    runs.hash(&mut hasher);
    hasher.finish()
}
