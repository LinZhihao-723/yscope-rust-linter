//! End-to-end tests of the `yscope-rust-linter` binary.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;

const INVALID_SUPPRESSION: &str = "// yscope-rust-linter: allow(errors-doc)\n";

/// Creates an empty directory for one test under the target's temporary directory.
///
/// # Returns
///
/// The path of the directory.
///
/// # Panics
///
/// Panics if the directory can't be recreated.
fn test_directory(test_name: &str) -> PathBuf {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("cli")
        .join(test_name);
    if directory.exists() {
        fs::remove_dir_all(&directory).expect("failed to clear the test directory");
    }
    fs::create_dir_all(&directory).expect("failed to create the test directory");
    directory
}

/// Writes `contents` to `relative_path` under `directory`, creating parent directories.
///
/// # Panics
///
/// Panics if the file can't be written.
fn write_file(directory: &Path, relative_path: &str, contents: &str) {
    let path = directory.join(relative_path);
    fs::create_dir_all(path.parent().expect("a file path has a parent"))
        .expect("failed to create the parent directory");
    fs::write(path, contents).expect("failed to write the test file");
}

/// Runs the binary in `directory` with `args`, feeding `stdin` to it.
///
/// # Returns
///
/// The finished process's output.
///
/// # Panics
///
/// Panics if the binary can't be run.
fn run(directory: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_yscope-rust-linter"))
        .args(args)
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn the binary");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(stdin.as_bytes())
        .expect("failed to write stdin");
    child
        .wait_with_output()
        .expect("failed to wait for the binary")
}

fn stdout_lines(output: &Output) -> Vec<String> {
    String::from_utf8(output.stdout.clone())
        .expect("stdout is UTF-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn clean_directory_exits_zero_with_empty_stdout() {
    let directory = test_directory("clean_directory_exits_zero_with_empty_stdout");
    write_file(&directory, "src/lib.rs", "// A comment.\nfn f() {}\n");
    let output = run(&directory, &["src"], "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout_lines(&output), Vec::<String>::new());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "yscope-rust-linter: 0 diagnostics in 1 file checked, 0 generated files skipped\n"
    );
}

#[test]
fn diagnostics_are_sorted_and_exit_one() {
    let directory = test_directory("diagnostics_are_sorted_and_exit_one");
    write_file(&directory, "b.rs", INVALID_SUPPRESSION);
    write_file(
        &directory,
        "a.rs",
        &format!("fn f() {{}}\n{INVALID_SUPPRESSION}\n{INVALID_SUPPRESSION}"),
    );
    let output = run(&directory, &["."], "");
    assert_eq!(output.status.code(), Some(1));
    let message = "invalid-suppression: malformed suppression; expected `// yscope-rust-linter: \
                   allow(<rule-id>[, <rule-id>]...): <reason>`";
    assert_eq!(
        stdout_lines(&output),
        [
            format!("a.rs:2:1: {message}"),
            format!("a.rs:4:1: {message}"),
            format!("b.rs:1:1: {message}")
        ]
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "yscope-rust-linter: 3 diagnostics in 2 files checked, 0 generated files skipped\n"
    );
}

#[test]
fn directory_walk_skips_target_hidden_and_generated_files() {
    let directory = test_directory("directory_walk_skips_target_hidden_and_generated_files");
    write_file(&directory, "target/skipped.rs", INVALID_SUPPRESSION);
    write_file(&directory, ".hidden/skipped.rs", INVALID_SUPPRESSION);
    write_file(
        &directory,
        "src/generated.rs",
        &format!("// @generated\n{INVALID_SUPPRESSION}"),
    );
    write_file(&directory, "src/notes.txt", INVALID_SUPPRESSION);
    write_file(&directory, "src/checked.rs", INVALID_SUPPRESSION);
    let output = run(&directory, &["."], "");
    let paths: Vec<_> = stdout_lines(&output)
        .into_iter()
        .map(|line| {
            line.split(':')
                .next()
                .expect("a diagnostic has a path")
                .to_owned()
        })
        .collect();
    assert_eq!(paths, ["src/checked.rs"]);

    let output = run(&directory, &["target/skipped.rs", ".hidden/skipped.rs"], "");
    assert_eq!(stdout_lines(&output).len(), 2);
}

#[test]
fn rule_selection_hides_unused_suppressions_of_disabled_rules() {
    let directory = test_directory("rule_selection_hides_unused_suppressions_of_disabled_rules");
    write_file(
        &directory,
        "a.rs",
        "// yscope-rust-linter: allow(errors-doc): reason\nfn f() {}\n",
    );
    let output = run(&directory, &["a.rs"], "");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout_lines(&output),
        [
            "a.rs:1:1: unused-suppression: this suppression of errors-doc covers no finding on \
             line 2; remove it"
        ]
    );

    let output = run(&directory, &["--rule", "comment-greedy-wrap", "a.rs"], "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn width_flag_changes_the_fill_threshold() {
    let directory = test_directory("width_flag_changes_the_fill_threshold");
    write_file(&directory, "a.rs", "// aaaa\n// bbb\n");
    let output = run(&directory, &["--width", "10", "a.rs"], "");
    assert_eq!(output.status.code(), Some(0));

    let output = run(&directory, &["--width", "11", "a.rs"], "");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout_lines(&output),
        [
            "a.rs:1:1: comment-greedy-wrap: line is 7 columns wide and the next line's first word \
             `bbb` (3 columns) fits within 11; move it up"
        ]
    );
}

#[test]
fn include_tests_flag_checks_test_fns() {
    let directory = test_directory("include_tests_flag_checks_test_fns");
    write_file(
        &directory,
        "a.rs",
        "/// Checks.\n///\n/// # Errors\n///\n/// Returns an error if:\n///\n/// * Forwards \
         [`load`]'s return values on failure.\n#[test]\nfn checks() -> Result<(), E> {\n    \
         Ok(())\n}\n",
    );
    let output = run(&directory, &["a.rs"], "");
    assert_eq!(output.status.code(), Some(0));

    let output = run(&directory, &["--include-tests", "a.rs"], "");
    assert_eq!(
        stdout_lines(&output),
        [
            "a.rs:7:1: errors-doc: \"Forwards [`load`]\" names a function whose errors `checks` \
             never forwards"
        ]
    );
}

#[test]
fn symbol_order_skips_out_of_line_test_modules_unless_tests_are_included() {
    let directory =
        test_directory("symbol_order_skips_out_of_line_test_modules_unless_tests_are_included");
    write_file(&directory, "src/lib.rs", "#[cfg(test)]\nmod tests;\n");
    write_file(&directory, "src/tests.rs", "fn a() {}\npub fn b() {}\n");
    let output = run(&directory, &["--rule", "symbol-order", "src"], "");
    assert_eq!(output.status.code(), Some(0));

    let output = run(
        &directory,
        &["--rule", "symbol-order", "--include-tests", "src"],
        "",
    );
    assert_eq!(
        stdout_lines(&output),
        [
            "src/tests.rs:2:1: symbol-order: pub fn `b` should come before private fn `a`: pub \
             items come before private items"
        ]
    );
}

#[test]
fn undecidable_test_module_is_one_parse_error_that_names_the_asking_rules() {
    let directory =
        test_directory("undecidable_test_module_is_one_parse_error_that_names_the_asking_rules");
    write_file(&directory, "dir/mod.rs", "fn broken( {}\n");
    write_file(&directory, "dir/child.rs", "fn f() {}\n");
    let problem = "dir/child.rs:1:1: parse-error: can't tell whether this file is a \
                   `#[cfg(test)]` module: syn failed to parse `dir/mod.rs` (cannot parse string \
                   into token stream);";
    assert_eq!(
        stdout_lines(&run(&directory, &["dir/child.rs"], "")),
        [format!(
            "{problem} errors-doc, symbol-order, trait-params-doc, type-params-doc check it as \
             non-test code"
        )]
    );
    assert_eq!(
        stdout_lines(&run(
            &directory,
            &["--rule", "errors-doc", "dir/child.rs"],
            ""
        )),
        [format!("{problem} errors-doc checks it as non-test code")]
    );
    assert_eq!(
        stdout_lines(&run(&directory, &["--include-tests", "dir/child.rs"], "")),
        Vec::<String>::new()
    );
}

#[test]
fn line_ranges_and_excludes_filter_files_and_lines() {
    let directory = test_directory("line_ranges_and_excludes_filter_files_and_lines");
    let source_text = format!("{INVALID_SUPPRESSION}\n\n{INVALID_SUPPRESSION}");
    write_file(&directory, "a.rs", &source_text);
    write_file(&directory, "excluded/b.rs", INVALID_SUPPRESSION);

    let output = run(
        &directory,
        &[
            "a.rs:3-4",
            "a.rs:10",
            "excluded/b.rs",
            "--exclude",
            "./excluded",
        ],
        "",
    );
    let lines: Vec<_> = stdout_lines(&output)
        .into_iter()
        .map(|line| {
            line.split(": ")
                .next()
                .expect("a diagnostic has a location")
                .to_owned()
        })
        .collect();
    assert_eq!(lines, ["a.rs:4:1"]);

    let output = run(&directory, &["a.rs:3-4", "a.rs"], "");
    assert_eq!(stdout_lines(&output).len(), 2);
}

#[test]
fn diff_input_selects_changed_lines_and_respects_excludes() {
    let directory = test_directory("diff_input_selects_changed_lines_and_respects_excludes");
    write_file(
        &directory,
        "a.rs",
        &format!("{INVALID_SUPPRESSION}\n{INVALID_SUPPRESSION}"),
    );
    write_file(&directory, "b.rs", INVALID_SUPPRESSION);
    let diff_text = "\
--- a/a.rs
+++ b/a.rs
@@ -3,2 +2,0 @@
-// deleted
-// deleted
--- a/b.rs
+++ b/b.rs
@@ -1 +1 @@
-old
+// yscope-rust-linter: allow(errors-doc)
";
    let output = run(&directory, &["--diff", "-", "--exclude", "b.rs"], diff_text);
    let lines: Vec<_> = stdout_lines(&output)
        .into_iter()
        .map(|line| {
            line.split(": ")
                .next()
                .expect("a diagnostic has a location")
                .to_owned()
        })
        .collect();
    assert_eq!(lines, ["a.rs:3:1"]);
}

#[test]
fn diff_that_deletes_a_forwards_bullet_reports_the_missing_forward() {
    let directory =
        test_directory("diff_that_deletes_a_forwards_bullet_reports_the_missing_forward");
    write_file(
        &directory,
        "src/lib.rs",
        "/// Loads.\n///\n/// # Errors\n///\n/// Returns an error if:\n///\n/// * Forwards \
         [`open`]'s return values on failure.\npub fn load() -> Result<(), E> {\n    open()?;\n    \
         read()?;\n    Ok(())\n}\n",
    );
    let diff_text = "\
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -8 +7,0 @@
-/// * Forwards [`read`]'s return values on failure.
";
    let output = run(&directory, &["--diff", "-"], diff_text);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout_lines(&output),
        [
            "src/lib.rs:10:11: errors-doc: `load` forwards errors from `read`, but `# Errors` \
             does not list it; add \"* Forwards [`read`]'s return values on failure.\""
        ]
    );
}

#[test]
fn diff_that_inserts_a_misplaced_item_reports_the_item_it_displaces() {
    let directory =
        test_directory("diff_that_inserts_a_misplaced_item_reports_the_item_it_displaces");
    write_file(
        &directory,
        "src/lib.rs",
        "pub fn a() {}\nfn b() {}\npub fn c() {}\nmod m {\n    fn d() {}\n    pub fn e() {}\n}\n",
    );
    let diff_text = "\
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,0 +2 @@
+fn b() {}
";
    let output = run(
        &directory,
        &["--rule", "symbol-order", "--diff", "-"],
        diff_text,
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout_lines(&output),
        [
            "src/lib.rs:3:1: symbol-order: pub fn `c` should come after pub fn `a`: pub items \
             come before private items"
        ]
    );
}

#[test]
fn empty_diff_checks_nothing_and_exits_zero() {
    let directory = test_directory("empty_diff_checks_nothing_and_exits_zero");
    write_file(&directory, "a.rs", INVALID_SUPPRESSION);
    let output = run(&directory, &["--diff", "-"], "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout_lines(&output), Vec::<String>::new());
}

#[test]
fn github_format_renders_workflow_commands() {
    let directory = test_directory("github_format_renders_workflow_commands");
    write_file(
        &directory,
        "a.rs",
        "// yscope-rust-linter: allow(nope): reason\n",
    );
    let output = run(&directory, &["--format", "github", "a.rs"], "");
    assert_eq!(
        stdout_lines(&output),
        [
            "::error file=a.rs,line=1,col=1,title=invalid-suppression::unknown rule `nope`; \
             expected one of comment-greedy-wrap, errors-doc, symbol-order, trait-params-doc, \
             type-params-doc"
        ]
    );
}

#[test]
fn fatal_errors_exit_two_with_empty_stdout() {
    let directory = test_directory("fatal_errors_exit_two_with_empty_stdout");
    write_file(&directory, "a.rs", INVALID_SUPPRESSION);
    write_file(&directory, "dir/b.rs", "");
    let cases: [&[&str]; 5] = [
        &[],
        &["a.rs", "missing.rs"],
        &["a.rs:0"],
        &["dir:1-2"],
        &["--rule", "unknown", "a.rs"],
    ];
    for args in cases {
        let output = run(&directory, args, "");
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(stdout_lines(&output), Vec::<String>::new(), "{args:?}");
    }
    for diff_text in [
        "+++ \"b/a b.rs\"\n",
        "+++ b/a.rs\n@@ -1 +0 @@\n",
        "+++ b/a.rs\n@@ -1 +18446744073709551615,2 @@\n",
    ] {
        let output = run(&directory, &["--diff", "-"], diff_text);
        assert_eq!(output.status.code(), Some(2), "{diff_text:?}");
        assert_eq!(stdout_lines(&output), Vec::<String>::new(), "{diff_text:?}");
    }
}

#[test]
fn version_flag_prints_the_package_version() {
    let directory = test_directory("version_flag_prints_the_package_version");
    let output = run(&directory, &["--version"], "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        stdout_lines(&output),
        [format!("yscope-rust-linter {}", env!("CARGO_PKG_VERSION"))]
    );
}

#[test]
fn param_doc_rules_honor_rule_selection_and_suppressions() {
    let directory = test_directory("param_doc_rules_honor_rule_selection_and_suppressions");
    write_file(
        &directory,
        "a.rs",
        concat!(
            "/// A cache.\n",
            "struct Cache<Value>(Value);\n",
            "\n",
            "trait Store {\n",
            "    /// Loads.\n",
            "    fn load(&self, key: &str);\n",
            "\n",
            "    // yscope-rust-linter: allow(trait-params-doc): reason\n",
            "    /// Saves.\n",
            "    fn save(&self, key: &str);\n",
            "}\n",
            "\n",
            "// yscope-rust-linter: allow-item(type-params-doc): reason\n",
            "/// A map.\n",
            "type Map<Key> = Vec<Key>;\n",
        ),
    );
    let type_params_finding = "a.rs:1:1: type-params-doc: struct `Cache` has generic parameters \
                               but no `# Type Parameters` section; add one with a bullet for each \
                               of `Value`";
    let trait_params_finding = "a.rs:5:5: trait-params-doc: trait method `load` has parameters \
                                but no `# Parameters` section; add one with a bullet for each of \
                                `key`";
    assert_eq!(
        stdout_lines(&run(&directory, &["a.rs"], "")),
        [type_params_finding, trait_params_finding]
    );
    assert_eq!(
        stdout_lines(&run(&directory, &["--rule", "type-params-doc", "a.rs"], "")),
        [type_params_finding]
    );
    assert_eq!(
        stdout_lines(&run(
            &directory,
            &["--rule", "trait-params-doc", "a.rs"],
            ""
        )),
        [trait_params_finding]
    );
}

#[test]
fn diff_that_adds_a_generic_parameter_reports_the_missing_bullet() {
    let directory = test_directory("diff_that_adds_a_generic_parameter_reports_the_missing_bullet");
    write_file(
        &directory,
        "src/lib.rs",
        "/// Loads.\n///\n/// # Type Parameters\n///\n/// * `Value` - The value.\npub fn \
         load<Value, Codec>() {}\n\n/// Saves.\npub fn save<Value>() {}\n",
    );
    let diff_text = "\
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -6 +6 @@
-pub fn load<Value>() {}
+pub fn load<Value, Codec>() {}
";
    let output = run(&directory, &["--diff", "-"], diff_text);
    assert_eq!(
        stdout_lines(&output),
        [
            "src/lib.rs:3:1: type-params-doc: `# Type Parameters` doesn't document `Codec`; add \
             \"* `Codec` - ...\""
        ]
    );
}
