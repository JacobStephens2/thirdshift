//! Email codes and their durable limits share the account store's lock.
use super::*;

#[derive(Default, Deserialize, Serialize)]
pub(super) struct Codes {
    pending: Option<Code>,
    sent: Vec<i64>,
}

#[derive(Deserialize, Serialize)]
struct Code {
    hash: String,
    sent_at: i64,
    attempts: u8,
}

impl Codes {
    pub(super) fn clear(&mut self) -> bool {
        self.pending.take().is_some()
    }
}

impl Accounts {
    /// The sender crosses the external email seam. Reserve the limits before
    /// calling it so even a failed send cannot be retried without a bound.
    pub fn send_code(&mut self, email: &str, send: impl FnOnce(&str) -> Result<()>) -> Result<()> {
        if !self.allowed.iter().any(|allowed| allowed == email) {
            return Ok(());
        }
        let account = self.accounts.entry(email.into()).or_default();
        account
            .codes
            .sent
            .retain(|sent| self.now.saturating_sub(*sent) < 24 * 60 * 60);
        if account.codes.sent.len() >= 10
            || account
                .codes
                .sent
                .last()
                .is_some_and(|sent| self.now.saturating_sub(*sent) < 60)
        {
            return Ok(());
        }
        // Rejection sampling keeps all six-digit strings equally likely.
        let number = loop {
            let number = OsRng.next_u32();
            if number < u32::MAX - u32::MAX % 1_000_000 {
                break number % 1_000_000;
            }
        };
        let code = format!("{number:06}");
        account.codes.pending = Some(Code {
            hash: hash_secret(&code)?,
            sent_at: self.now,
            attempts: 0,
        });
        account.codes.sent.push(self.now);
        self.save()?;
        send(&code)?;
        self.record(email, EventKind::CodeSent)
    }

    pub fn set_password_by_code(
        &mut self,
        email: &str,
        code: &str,
        password: &str,
    ) -> Result<SignIn> {
        if password.is_empty() || !self.allowed.iter().any(|allowed| allowed == email) {
            return Ok(SignIn::Refused);
        }
        let Some(pending) = self
            .accounts
            .get_mut(email)
            .and_then(|account| account.codes.pending.as_mut())
        else {
            return Ok(SignIn::Refused);
        };
        if self.now.saturating_sub(pending.sent_at) >= 15 * 60 || pending.attempts >= 5 {
            return Ok(SignIn::Refused);
        }
        pending.attempts += 1;
        let valid = matches_secret(code, &pending.hash);
        self.save()?;
        if !valid {
            return Ok(SignIn::Refused);
        }
        self.set_password(email, password)?;
        self.accounts.get_mut(email).expect("code account").verified = true;
        self.start_session(email)
    }
}
