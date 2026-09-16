pub mod geometry;
pub mod keys;
pub mod presentation;
pub mod shortcut;

pub use geometry::{Point, Rect, Size};
pub use presentation::{PresentationPhase, PresentationState, PresentationTransition};
pub use shortcut::{
    ConflictCandidate, QuickTerminalShortcut, RegistrationIdentity, ShortcutConflict,
};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]
mod tests {
    fn portable<T: Clone + Send + Sync + 'static>() {}

    #[test]
    fn quick_terminal_public_contracts_are_portable() {
        portable::<super::QuickTerminalShortcut>();
        portable::<super::RegistrationIdentity>();
        portable::<super::Point>();
        portable::<super::Rect>();
        portable::<super::Size>();
        portable::<super::PresentationPhase>();
        portable::<super::PresentationState>();
        portable::<super::PresentationTransition>();
    }
}
