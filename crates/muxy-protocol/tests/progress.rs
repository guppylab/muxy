use muxy_protocol::wire::{Decoder, encode};
use muxy_protocol::{
    CONTROL, ErrorCode, Message, ProgressState, SessionId, SessionProgress, TerminalProgress,
};

#[test]
fn progress_states_and_clearing_roundtrip_with_bounded_percentages()
-> Result<(), Box<dyn std::error::Error>> {
    for state in [
        ProgressState::Running,
        ProgressState::Error,
        ProgressState::Indeterminate,
        ProgressState::Paused,
    ] {
        for percent in [None, Some(0), Some(42), Some(100), Some(101), Some(255)] {
            let message = Message::Progress {
                session: SessionId::new(1).ok_or("id")?,
                progress: SessionProgress {
                    progress: Some(TerminalProgress { state, percent }),
                    completed: 42,
                },
            };
            if percent.is_some_and(|percent| percent > 100) {
                assert_eq!(message.validate(), Err(ErrorCode::BadRequest));
                continue;
            }
            let mut bytes = Vec::new();
            encode(&message, CONTROL, &mut bytes)?;
            assert_eq!(Decoder::new(bytes.as_slice()).next()?, (CONTROL, message));
        }
    }
    let clear = Message::Progress {
        session: SessionId::new(1).ok_or("id")?,
        progress: SessionProgress {
            progress: None,
            completed: 43,
        },
    };
    let mut bytes = Vec::new();
    encode(&clear, CONTROL, &mut bytes)?;
    assert_eq!(Decoder::new(bytes.as_slice()).next()?, (CONTROL, clear));
    Ok(())
}
