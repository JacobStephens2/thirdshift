//! Recover guidance argv through a POSIX shell without starting the factory.

/// Recover exactly the argv a copied guidance command supplies.
pub fn words(command: &str) -> Vec<String> {
    let output = std::process::Command::new("sh")
        .args([
            "-c",
            &format!("thirdshift() {{ printf '%s\\0' \"$@\"; }}; {command}"),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let mut words = output.stdout.split(|byte| *byte == 0).collect::<Vec<_>>();
    assert_eq!(words.pop(), Some(&b""[..]));
    words
        .into_iter()
        .map(|word| String::from_utf8(word.to_vec()).unwrap())
        .collect()
}
