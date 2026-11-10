//! Unit tests have no Command to remove the shared extraction at finish.
//! Hold it for every test user, including isolated Session subprocesses.

use std::sync::{Mutex, PoisonError};

static USERS: Mutex<usize> = Mutex::new(0);

pub(crate) struct FactorySkills(());

impl FactorySkills {
    pub(crate) fn hold() -> Self {
        *USERS.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        Self(())
    }
}

impl Drop for FactorySkills {
    fn drop(&mut self) {
        let mut users = USERS.lock().unwrap_or_else(PoisonError::into_inner);
        *users -= 1;
        if *users == 0 {
            // Keep the user lock through removal so a new test cannot start
            // using an extraction the previous last user is removing.
            super::remove_written();
        }
    }
}
