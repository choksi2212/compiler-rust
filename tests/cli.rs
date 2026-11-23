use std::process::Command;
#[test]
fn all_demos_with_and_without_optimization() {
    for name in [
        "addition",
        "loop",
        "factorial",
        "pattern",
        "conditional",
        "strings",
    ] {
        let input = format!("examples/{name}.c");
        let expected = std::fs::read_to_string(format!("examples/{name}.expected"))
            .unwrap()
            .replace("\r\n", "\n");
        for opt in [true, false] {
            let mut c = Command::new(env!("CARGO_BIN_EXE_compiler"));
            c.arg(&input);
            if !opt {
                c.arg("--no-opt");
            }
            let output = c.output().unwrap();
            assert!(
                output.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                expected,
                "{name} optimized={opt}"
            );
        }
    }
}
#[test]
fn stage_dumps_and_cli_errors() {
    let output = Command::new(env!("CARGO_BIN_EXE_compiler"))
        .args([
            "examples/addition.c",
            "--tokens",
            "--ast",
            "--ir",
            "--optimized-ir",
            "--bytecode",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let dumps = String::from_utf8(output.stderr).unwrap();
    for label in ["TOKENS", "AST", "THREE-ADDRESS IR", "OPTIMIZED IR", "Push"] {
        assert!(dumps.contains(label));
    }
    assert!(
        !Command::new(env!("CARGO_BIN_EXE_compiler"))
            .arg("--invalid")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        !Command::new(env!("CARGO_BIN_EXE_compiler"))
            .arg("missing.c")
            .output()
            .unwrap()
            .status
            .success()
    );
}
