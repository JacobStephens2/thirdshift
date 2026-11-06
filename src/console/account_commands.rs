//! Console account commands use the same account store and log as HTTP.
use anyhow::{Result, bail, ensure};
use std::io::Write;

use super::accounts::Accounts;
use crate::config::UserConfig;

pub(crate) struct Scopes {
    email: Option<String>,
    scopes: Option<Vec<super::Scope>>,
    json: bool,
}

impl Scopes {
    pub(crate) fn parse(args: &[String]) -> Result<Self> {
        let usage = "usage: thirdshift console scopes [--email <address>] [--scopes read,security,act,run,admin] [--json]";
        let mut command = Self {
            email: None,
            scopes: None,
            json: false,
        };
        let mut args = args.iter();
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--email" if command.email.is_none() => {
                    command.email = Some(
                        args.next()
                            .filter(|email| !email.starts_with('-'))
                            .ok_or_else(|| anyhow::anyhow!(usage))?
                            .clone(),
                    );
                }
                "--scopes" if command.scopes.is_none() => {
                    command.scopes = Some(super::Scope::parse(
                        args.next().ok_or_else(|| anyhow::anyhow!(usage))?,
                    )?);
                }
                "--json" if !command.json => command.json = true,
                _ => bail!(usage),
            }
        }
        Ok(command)
    }

    pub(crate) fn run(self) -> Result<String> {
        let email = email(self.email.as_deref())?;
        let settings = if let Some(scopes) = self.scopes {
            crate::config::console::write_address_scopes(&email, scopes)?
        } else {
            UserConfig::load()?.console
        };
        ensure!(
            settings.allowed_emails.contains(&email),
            "address is not allowed in console.allowed_emails"
        );
        let scopes = settings.scopes_for(&email);
        Ok(if self.json {
            serde_json::json!({"email": email, "scopes": scopes}).to_string()
        } else {
            format!(
                "{email}: {}",
                scopes
                    .iter()
                    .map(|scope| scope.name())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        })
    }
}

fn email(given: Option<&str>) -> Result<String> {
    if let Some(email) = given {
        return Ok(email.into());
    }
    let config = UserConfig::load()?;
    if let [email] = &config.console.allowed_emails[..] {
        return Ok(email.clone());
    }
    bail!("give --email <address>; console.allowed_emails must name exactly one address to omit it")
}

pub fn password(given: Option<&str>) -> Result<String> {
    let email = email(given)?;
    ensure!(
        UserConfig::load()?.console.allowed_emails.contains(&email),
        "address is not allowed in console.allowed_emails"
    );
    crate::interrupt::install()?;
    let (first, second) = {
        // Suppress echo before either prompt, and keep it off between them.
        let _echo = crate::terminal::EchoOff::new()?;
        (prompt("Password: ")?, prompt("Confirm password: ")?)
    };
    ensure!(first == second, "passwords do not match");
    Accounts::load()?.set_password(&email, &first)?;
    Ok(format!(
        "Password set for {email}; previous sessions ended."
    ))
}

fn prompt(prompt: &str) -> Result<String> {
    eprint!("{prompt}");
    std::io::stderr().flush()?;
    let answer = crate::terminal::line()?.ok_or_else(|| anyhow::anyhow!("password input ended"))?;
    eprintln!();
    Ok(answer)
}

pub fn sign_out(given: Option<&str>, json: bool) -> Result<()> {
    let email = email(given)?;
    Accounts::load()?.sign_out(&email, None)?;
    if json {
        println!(
            "{}",
            serde_json::json!({ "email": email, "signed_out": true })
        );
    } else {
        println!("Signed out {email}.");
    }
    Ok(())
}

pub fn print_log(json: bool) -> Result<()> {
    let events = Accounts::load()?.events()?;
    if json {
        println!("{}", serde_json::to_string(&events)?);
    } else {
        for event in events {
            println!("{} {} {}", event.time, event.email, event.summary());
        }
    }
    Ok(())
}
