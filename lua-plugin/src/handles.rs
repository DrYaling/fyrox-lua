//! Stable, serializable handle identity passed across the Lua boundary.

use fyrox::core::pool::Handle;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HandleToken {
    pub index: u32,
    pub generation: u32,
}

impl HandleToken {
    #[inline]
    pub fn from_handle<T>(handle: Handle<T>) -> Self {
        Self {
            index: handle.index(),
            generation: handle.generation(),
        }
    }

    #[inline]
    pub fn to_handle<T>(self) -> Handle<T> {
        Handle::new(self.index, self.generation)
    }

    #[inline]
    pub fn matches<T>(self, handle: Handle<T>) -> bool {
        self.index == handle.index() && self.generation == handle.generation()
    }
}

#[cfg(test)]
mod tests {
    use super::HandleToken;
    use fyrox::core::pool::Handle;

    #[test]
    fn token_round_trips_handle_identity() {
        let handle = Handle::<u32>::new(7, 3);
        let token = HandleToken::from_handle(handle);
        assert_eq!(token.to_handle::<u32>(), handle);
        assert!(token.matches(handle));
        assert!(!token.matches(Handle::<u32>::new(7, 4)));
    }
}
