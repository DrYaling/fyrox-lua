//! Bounded diagnostics shared by the Lua runtime and host.
//!
//! Keeping this policy in one module prevents the VM and the engine adapter
//! from drifting into different error retention or message-size behavior.
use crate::handles::HandleToken;
use std::{
    collections::VecDeque,
    fmt::{self, Write},
    path::PathBuf,
};

pub(crate) const MAX_ERROR_REPORTS: usize = 256;
const MAX_ERROR_MESSAGE_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LuaErrorReport {
    pub phase: String,
    pub script: Option<PathBuf>,
    pub instance_id: Option<u64>,
    pub scope: Option<HandleToken>,
    pub frame: u64,
    pub message: String,
    pub recovered: bool,
}

#[derive(Debug, Default)]
pub(crate) struct ErrorBuffer(VecDeque<LuaErrorReport>);

impl ErrorBuffer {
    #[inline]
    pub fn push(&mut self, report: LuaErrorReport) {
        if self.0.len() >= MAX_ERROR_REPORTS {
            self.0.pop_front();
        }
        self.0.push_back(report);
    }

    #[inline]
    pub fn drain(&mut self) -> Vec<LuaErrorReport> {
        self.0.drain(..).collect()
    }
}

#[inline]
pub(crate) fn bounded_message(error: impl std::fmt::Display) -> String {
    struct BoundedMessage {
        value: String,
        truncated: bool,
    }

    impl Write for BoundedMessage {
        fn write_str(&mut self, value: &str) -> fmt::Result {
            let remaining = MAX_ERROR_MESSAGE_BYTES.saturating_sub(self.value.len());
            if value.len() <= remaining {
                self.value.push_str(value);
                return Ok(());
            }

            let mut end = remaining;
            while !value.is_char_boundary(end) {
                end -= 1;
            }
            self.value.push_str(&value[..end]);
            self.truncated = true;
            Err(fmt::Error)
        }
    }

    let mut message = BoundedMessage {
        value: String::with_capacity(MAX_ERROR_MESSAGE_BYTES + 3),
        truncated: false,
    };
    let _ = write!(&mut message, "{error}");
    if message.truncated {
        message.value.push_str("...");
    }
    message.value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_is_bounded_fifo() {
        let mut buffer = ErrorBuffer::default();
        for frame in 0..(MAX_ERROR_REPORTS + 3) {
            buffer.push(LuaErrorReport {
                phase: "test".into(),
                script: None,
                instance_id: None,
                scope: None,
                frame: frame as u64,
                message: "x".into(),
                recovered: true,
            });
        }
        let reports = buffer.drain();
        assert_eq!(reports.len(), MAX_ERROR_REPORTS);
        assert_eq!(reports.first().unwrap().frame, 3);
        assert_eq!(
            reports.last().unwrap().frame,
            (MAX_ERROR_REPORTS + 2) as u64
        );
    }

    #[test]
    fn message_limit_preserves_utf8_boundary() {
        let message = "界".repeat(5000);
        let bounded = bounded_message(message);
        assert!(bounded.len() <= MAX_ERROR_MESSAGE_BYTES + "…".len());
        assert!(bounded.ends_with("..."));
    }
}
