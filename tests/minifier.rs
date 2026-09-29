use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
use tree_trimmer::{compress, equivalent, render, tokenize};

#[test]
fn preserves_preprocessing_token_boundaries() {
    let atoms = [
        "a", "L", "u8", "0", "1e", ".1e", "0x1p", ".", "+", "-", "/", "*", "<", ":", "%", ">", "=",
        "?", "&", "|", "++", "--", "\"text\"", "'x'", "L'x'",
    ];
    for a in atoms {
        for b in atoms {
            let source = format!("{a} {b}");
            let tokens = tokenize(&source).unwrap();
            assert_eq!(tokenize(&render(&tokens)).unwrap(), tokens, "{source}");
        }
    }
}

#[test]
fn comments_escapes_splices_and_directives() {
    let source = "#define F(x) x \\\n + 1\nint/**/value = F(2); // comment\nchar *s = \"/*quoted*/\\\"\"; char c = '\\\'';\n";
    let tokens = tokenize(source).unwrap();
    let compact = render(&tokens);
    assert!(compact.starts_with("#define F(x) x  + 1\n"));
    assert!(compact.contains("int value"));
    assert!(compact.contains("/*quoted*/"));
    assert_eq!(tokenize(&compact).unwrap(), tokens);
    assert!(tokenize("/* unterminated").is_err());
    assert!(tokenize("\"unterminated").is_err());
    assert!(tokenize("??/").is_err());
}

#[test]
fn verification_checks_literals_and_pragmas() {
    equivalent("int x = 1;", "int x=1;").unwrap();
    assert!(equivalent("char*s=\"a b\";", "char*s=\"ab\";").is_err());
    assert!(equivalent("#pragma pack(1)\nint x;", "int x;").is_err());
    assert!(equivalent("int x;", "int x; int y;").is_err());
}

fn parser() -> String {
    let mut s = String::from(
        "#include \"tree_sitter/parser.h\"\nenum { sym_very_long_identifier_name=1 };\nstatic int lex(int lookahead, int state) { switch(state) {\n",
    );
    for n in 0..80 {
        s.push_str(&format!("case {n}: if (lookahead == 'a' || lookahead == 'A') return sym_very_long_identifier_name; break;\n"));
    }
    s.push_str("} return 0; }\n");
    s
}

#[test]
fn compression_is_deterministic_and_beats_whitespace() {
    let source = parser();
    let a = compress(&source, "", 4).unwrap();
    let b = compress(&source, "", 4).unwrap();
    assert_eq!(a.source, b.source);
    assert!(a.source.len() < a.whitespace_bytes / 2);
    assert!(a.macros > 0);
}

static SERIAL: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new(source: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "tree-trimmer-test-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(path.join("tree_sitter")).unwrap();
        fs::write(path.join("tree_sitter/parser.h"), "/* fixture header */\n").unwrap();
        fs::write(path.join("parser.c"), source).unwrap();
        Self(path)
    }
    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_tree-trimmer"))
            .arg(self.0.join("parser.c"))
            .args(args)
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn cli_verifies_and_replaces_in_place() {
    let source = parser();
    let fixture = Fixture::new(&source);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            fixture.0.join("parser.c"),
            fs::Permissions::from_mode(0o640),
        )
        .unwrap();
    }
    let run = fixture.run(&[]);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let result = fs::read_to_string(fixture.0.join("parser.c")).unwrap();
    assert!(result.len() < source.len() / 2);
    assert!(String::from_utf8_lossy(&run.stderr).contains("tokens identical"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(fixture.0.join("parser.c"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
    }
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 2);
    let rerun = fixture.run(&[]);
    assert!(
        rerun.status.success(),
        "{}",
        String::from_utf8_lossy(&rerun.stderr)
    );
}

#[test]
fn cli_separate_output_and_defines() {
    let source = parser();
    let fixture = Fixture::new(&source);
    let output = fixture.0.join("min.c");
    let run = fixture.run(&[
        "-o",
        output.to_str().unwrap(),
        "--cpp-arg",
        "-Da=999",
        "--cpp-arg",
        "-Db=888",
    ]);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("parser.c")).unwrap(),
        source
    );
    assert!(fs::read_to_string(output).unwrap().len() < source.len());
}

#[test]
fn failures_leave_input_and_output_untouched() {
    for source in [
        "#include \"tree_sitter/parser.h\"\nint x = __LINE__;",
        "#include \"tree_sitter/parser.h\"\nint broken = ;",
    ] {
        let fixture = Fixture::new(source);
        let output = fixture.0.join("min.c");
        fs::write(&output, "existing output").unwrap();
        assert!(!fixture.run(&[]).status.success());
        assert!(
            !fixture
                .run(&["-o", output.to_str().unwrap()])
                .status
                .success()
        );
        assert_eq!(
            fs::read_to_string(fixture.0.join("parser.c")).unwrap(),
            source
        );
        assert_eq!(fs::read_to_string(output).unwrap(), "existing output");
    }
}

#[test]
fn stringification_changes_are_rejected_before_writing() {
    let mut source = String::from(
        "#include \"tree_sitter/parser.h\"\n#define SPELL(x) #x\nint very_long_identifier=0;\nconst char *s=SPELL(very_long_identifier);\nint f(void){\n",
    );
    for n in 0..50 {
        source.push_str(&format!("very_long_identifier += {n};\n"));
    }
    source.push_str("return very_long_identifier; }\n");
    let fixture = Fixture::new(&source);
    let run = fixture.run(&[]);
    assert!(!run.status.success());
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("equivalence failed"),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("parser.c")).unwrap(),
        source
    );
}
