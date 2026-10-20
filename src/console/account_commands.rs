//! Console account commands use the same account store and log as HTTP.
use anyhow::{Result, bail, ensure};
use std::io::Write;

use super::accounts::Accounts;
use crate::config::UserConfig;

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

pub fn sign_out(given: Option<&str>) -> Result<String> {
    let email = email(given)?;
    Accounts::load()?.sign_out(&email, None)?;
    Ok(format!("Signed out {email}."))
}

pub fn print_log(json: bool) -> Result<()> {
    let events = Accounts::load()?.events()?;
    if json {
        println!("{}", serde_json::to_string(&events)?);
    } else {
        for event in events {
            println!("{} {} {}", event.time, event.email, event.event);
        }
    }
    Ok(())
}
