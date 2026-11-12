use std::process::Command;

#[test]
fn informational_flags_work_without_a_graphical_backend() {
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_fflume"))
            .env("SLINT_BACKEND", "invalid-test-backend")
            .args(args)
            .output()
            .unwrap()
    };

    let version = run(&["--version"]);
    assert!(version.status.success());
    assert!(version.stderr.is_empty());
    assert_eq!(
        version.stdout,
        concat!("fflume ", env!("CARGO_PKG_VERSION"), "\n").as_bytes()
    );

    let help = run(&["--help"]);
    assert!(help.status.success());
    assert!(help.stderr.is_empty());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Usage: fflume"));

    for args in [&["--unknown"][..], &["--version", "extra"]] {
        let invalid = run(args);
        assert_eq!(invalid.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&invalid.stderr).contains("Usage: fflume"));
    }
}
