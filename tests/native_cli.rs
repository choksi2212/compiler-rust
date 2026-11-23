#![cfg(all(windows, target_arch = "x86_64"))]
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};
static ID: AtomicUsize = AtomicUsize::new(0);
struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "native CLI Ω {} {}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn cli(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_compiler"))
            .current_dir(&self.0)
            .env("PATH", "")
            .args(args)
            .output()
            .unwrap()
    }
    fn source(&self) {
        fs::write(
            self.0.join("source.c"),
            "int main(){printf(\"hello 世界\\n\");return 0;}",
        )
        .unwrap();
    }
    fn clean_temps(&self) {
        assert!(
            fs::read_dir(&self.0).unwrap().all(|p| !p
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp"))
        );
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn code(out: &Output, expected: i32) {
    assert_eq!(
        out.status.code(),
        Some(expected),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn native_compiles_all_six_demos_both_modes_without_running_them() {
    let w = Workspace::new();
    for name in [
        "addition",
        "loop",
        "factorial",
        "pattern",
        "conditional",
        "strings",
    ] {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("examples/{name}.c"));
        let expected = fs::read_to_string(source.with_extension("expected"))
            .unwrap()
            .replace("\r\n", "\n");
        for no_opt in [false, true] {
            let dest = format!("{name}-{no_opt}.exe");
            let mut args = vec![source.to_str().unwrap(), "-o", &dest];
            if no_opt {
                args.push("--no-opt");
            }
            let out = w.cli(&args);
            code(&out, 0);
            assert!(out.stdout.is_empty());
            assert!(out.stderr.is_empty());
            let out = Command::new(w.0.join(dest))
                .current_dir(&w.0)
                .env("PATH", "")
                .output()
                .unwrap();
            code(&out, 0);
            assert_eq!(
                String::from_utf8(out.stdout).unwrap().replace("\r\n", "\n"),
                expected
            );
            assert!(out.stderr.is_empty());
        }
    }
    w.clean_temps();
}

#[test]
fn options_exit_codes_dumps_unicode_paths_and_default_vm() {
    let w = Workspace::new();
    w.source();
    for args in [
        vec![],
        vec!["--help"],
        vec!["--version"],
        vec!["--print-capabilities"],
    ] {
        code(&w.cli(&args), 0);
    }
    let caps = w.cli(&["--print-capabilities"]);
    let caps = String::from_utf8(caps.stdout).unwrap();
    assert!(caps.contains("\"full_c11\":false"));
    assert!(caps.contains("0.2.0"));
    for args in [
        vec!["--invalid"],
        vec!["-o"],
        vec!["source.c", "-o", "x", "-o", "y"],
        vec!["source.c", "other.c"],
        vec!["source.c", "--overwrite"],
        vec!["source.c", "--emit=machine"],
        vec!["source.c", "-o", "x", "--run-vm"],
        vec!["source.c", "-o", "x", "--bytecode"],
        vec!["--version", "source.c"],
    ] {
        code(&w.cli(&args), 2);
    }
    for args in [
        vec!["source.c"],
        vec!["source.c", "--run-vm"],
        vec!["source.c", "--no-opt"],
    ] {
        let out = w.cli(&args);
        code(&out, 0);
        assert_eq!(out.stdout, "hello 世界\n".as_bytes());
    }
    fs::rename(w.0.join("source.c"), w.0.join("-source Ω.c")).unwrap();
    let out = w.cli(&[
        "-o",
        "result 世界.exe",
        "--tokens",
        "--ast",
        "--ir",
        "--optimized-ir",
        "--emit=machine",
        "--",
        "-source Ω.c",
    ]);
    code(&out, 0);
    assert!(out.stdout.is_empty());
    let dump = String::from_utf8(out.stderr).unwrap();
    for label in [
        "TOKENS",
        "AST",
        "THREE-ADDRESS IR",
        "OPTIMIZED IR",
        "MACHINE CODE",
        "CallImport",
        "00001000",
    ] {
        assert!(dump.contains(label), "{label}");
    }
    let out = Command::new(w.0.join("result 世界.exe"))
        .env("PATH", "")
        .output()
        .unwrap();
    code(&out, 0);
    assert_eq!(out.stdout, "hello 世界\n".as_bytes());
}

#[test]
fn destination_safety_preserves_sources_compiler_and_old_output() {
    let w = Workspace::new();
    w.source();
    let original = fs::read(w.0.join("source.c")).unwrap();
    for name in ["source.c", "SOURCE.C", "./source.c"] {
        let out = w.cli(&["source.c", "-o", name, "--overwrite"]);
        code(&out, 1);
        assert!(String::from_utf8_lossy(&out.stderr).contains("protected"));
    }
    fs::hard_link(w.0.join("source.c"), w.0.join("source-alias.exe")).unwrap();
    code(
        &w.cli(&["source.c", "-o", "source-alias.exe", "--overwrite"]),
        1,
    );
    assert_eq!(fs::read(w.0.join("source.c")).unwrap(), original);
    let compiler = fs::canonicalize(env!("CARGO_BIN_EXE_compiler")).unwrap();
    code(
        &w.cli(&["source.c", "-o", compiler.to_str().unwrap(), "--overwrite"]),
        1,
    );
    // A compiler hard-link requires the same volume; put it beside the tested binary.
    let alias = compiler.with_file_name(format!(
        "compiler-protected-alias-{}.exe",
        std::process::id()
    ));
    fs::hard_link(&compiler, &alias).unwrap();
    let out = w.cli(&["source.c", "-o", alias.to_str().unwrap(), "--overwrite"]);
    fs::remove_file(alias).unwrap();
    code(&out, 1);
    for name in [
        "NUL",
        "con.exe",
        "COM1.exe",
        "LPT9",
        "COM¹.exe",
        "bad.exe.",
        "bad.exe ",
        "out:stream",
        "bad?.exe",
        "missing/out.exe",
        ".",
        "folder/",
    ] {
        code(&w.cli(&["source.c", "-o", name, "--overwrite"]), 1);
    }
    fs::create_dir(w.0.join("folder")).unwrap();
    code(&w.cli(&["source.c", "-o", "folder", "--overwrite"]), 1);
    code(&w.cli(&["missing.c", "-o", "out.exe"]), 1);
    fs::write(w.0.join("out.exe"), b"previous output").unwrap();
    code(&w.cli(&["source.c", "-o", "out.exe"]), 1);
    fs::write(w.0.join("bad.c"), "int main(){@}").unwrap();
    code(&w.cli(&["bad.c", "-o", "out.exe", "--overwrite"]), 1);
    assert_eq!(fs::read(w.0.join("out.exe")).unwrap(), b"previous output");
    let mut perms = fs::metadata(w.0.join("out.exe")).unwrap().permissions();
    let original_permissions = perms.clone();
    perms.set_readonly(true);
    fs::set_permissions(w.0.join("out.exe"), perms).unwrap();
    code(&w.cli(&["source.c", "-o", "out.exe", "--overwrite"]), 1);
    assert_eq!(fs::read(w.0.join("out.exe")).unwrap(), b"previous output");
    fs::set_permissions(w.0.join("out.exe"), original_permissions).unwrap();
    code(&w.cli(&["source.c", "-o", "out.exe", "--overwrite"]), 0);
    assert!(fs::read(w.0.join("out.exe")).unwrap().starts_with(b"MZ"));
    w.clean_temps();
}

#[test]
fn publication_rechecks_races_and_rejects_invalid_pe_without_partial_output() {
    use compiler::files::OutputTarget;
    let w = Workspace::new();
    w.source();
    let path = w.0.join("out.exe");
    let protected = [w.0.join("source.c")];
    let target = OutputTarget::prepare(&path, &protected, false).unwrap();
    assert!(target.publish(b"invalid PE").is_err());
    assert!(!path.exists());
    let mut context = compiler::context::CompilationContext::default();
    let id = context.add_source("unit.c", "int main(){return 0;}");
    let image = context
        .compile_native(id, &Default::default())
        .unwrap()
        .image
        .bytes;
    fs::write(&path, b"concurrent creator").unwrap();
    assert!(target.publish(&image).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"concurrent creator");
    let target = OutputTarget::prepare(&path, &protected, true).unwrap();
    fs::remove_file(&path).unwrap();
    fs::hard_link(&protected[0], &path).unwrap();
    assert!(target.publish(&image).is_err());
    assert_eq!(fs::read(&path).unwrap(), fs::read(&protected[0]).unwrap());
    w.clean_temps();
}

#[test]
fn bounded_file_input_rejects_oversize_bad_utf8_and_directories() {
    let w = Workspace::new();
    fs::write(
        w.0.join("source.c"),
        vec![b' '; compiler::lexer::MAX_SOURCE_BYTES + 1],
    )
    .unwrap();
    let out = w.cli(&["source.c", "-o", "out.exe"]);
    code(&out, 1);
    assert!(String::from_utf8_lossy(&out.stderr).contains("16MiB"));
    fs::write(w.0.join("source.c"), [0xff]).unwrap();
    code(&w.cli(&["source.c"]), 1);
    code(&w.cli(&[".", "-o", "out.exe"]), 1);
    assert!(!w.0.join("out.exe").exists());
    w.clean_temps();
}

#[test]
fn successful_nonzero_returns_flush_stdout_without_runtime_diagnostics() {
    let w = Workspace::new();
    for result in [1, 2, 7, 27, -1, i32::MIN] {
        let expression = if result == i32::MIN {
            "-2147483647-1".into()
        } else {
            result.to_string()
        };
        fs::write(
            w.0.join("source.c"),
            format!("int main(){{printf(\"visible\");return {expression};}}"),
        )
        .unwrap();
        for flag in ["--run-vm", "--no-opt"] {
            let out = w.cli(&["source.c", flag]);
            code(&out, result);
            assert_eq!(out.stdout, b"visible");
            assert!(out.stderr.is_empty());
        }
        code(&w.cli(&["source.c", "-o", "program.exe", "--overwrite"]), 0);
        let out = Command::new(w.0.join("program.exe"))
            .env("PATH", "")
            .output()
            .unwrap();
        code(&out, result);
        assert_eq!(out.stdout, b"visible");
        assert!(out.stderr.is_empty());
    }
    w.clean_temps();
}

#[test]
fn explicit_typed_mode_syntax_only_and_mode_conflicts() {
    let w = Workspace::new();
    fs::write(w.0.join("source.c"),"typedef unsigned long long U;int main(void){U x=18446744073709551615ULL;return (int)(x>>62);}").unwrap();
    for optimization in ["-O0", "-O1", "--no-opt"] {
        let out = w.cli(&[
            "source.c",
            "--std=c11",
            "-o",
            "typed.exe",
            "--overwrite",
            optimization,
            "--tokens",
            "--ast",
            "--ir",
            "--optimized-ir",
            "--emit=machine",
        ]);
        code(&out, 0);
        assert!(out.stdout.is_empty());
        let dumps = String::from_utf8(out.stderr).unwrap();
        for label in [
            "TOKENS",
            "TYPED AST",
            "TYPED IR",
            "OPTIMIZED TYPED IR",
            "MACHINE CODE",
        ] {
            assert!(dumps.contains(label));
        }
        let out = Command::new(w.0.join("typed.exe"))
            .env("PATH", "")
            .output()
            .unwrap();
        code(&out, 3);
        assert!(out.stderr.is_empty());
    }
    code(
        &w.cli(&[
            "source.c",
            "--std=c11",
            "-fsyntax-only",
            "--tokens",
            "--ast",
        ]),
        0,
    );
    fs::write(
        w.0.join("types.c"),
        "typedef int (*F)(int);typedef unsigned long U;",
    )
    .unwrap();
    code(&w.cli(&["types.c", "--std=c11", "-fsyntax-only"]), 0);
    for args in [
        vec!["source.c", "--std=c11"],
        vec!["source.c", "-fsyntax-only"],
        vec!["source.c", "--std=c11", "--run-vm"],
        vec!["source.c", "--std=c11", "--bytecode"],
        vec![
            "source.c",
            "--std=c11",
            "--semantics=teaching",
            "-o",
            "x.exe",
        ],
        vec!["source.c", "--std=c11", "-fsyntax-only", "-o", "x.exe"],
        vec!["source.c", "--std=c11", "-fsyntax-only", "--ir"],
        vec!["source.c", "-O0", "-O1"],
        vec!["source.c", "--no-opt", "-O1"],
    ] {
        code(&w.cli(&args), 2);
    }
    // No typed fallback is hidden behind the default VM or teaching compiler.
    code(&w.cli(&["source.c", "--semantics=teaching"]), 1);
    fs::write(w.0.join("bad.c"), "int main(){int a[];return sizeof(a);}").unwrap();
    fs::write(w.0.join("old.exe"), b"preserve").unwrap();
    code(
        &w.cli(&["bad.c", "--std=c11", "-o", "old.exe", "--overwrite"]),
        1,
    );
    assert_eq!(fs::read(w.0.join("old.exe")).unwrap(), b"preserve");
    w.clean_temps();
}

#[test]
fn nested_expression_tails_report_limits_instead_of_overflowing_the_stack() {
    let w = Workspace::new();
    let mut expression = "0".to_string();
    for _ in 0..32 {
        expression = format!("({expression}+{})", vec!["0"; 126].join("+"));
    }
    fs::write(
        w.0.join("source.c"),
        format!("int main(){{int x={expression};return 0;}}"),
    )
    .unwrap();
    for flags in [
        vec![],
        vec!["--std=c11", "-fsyntax-only"],
        vec!["-o", "out.exe"],
    ] {
        let mut args = vec!["source.c"];
        args.extend(flags);
        let out = w.cli(&args);
        code(&out, 1);
        assert!(String::from_utf8_lossy(&out.stderr).contains("expression size limit"));
        assert!(!String::from_utf8_lossy(&out.stderr).contains("overflowed its stack"));
        assert!(!w.0.join("out.exe").exists());
    }
}
