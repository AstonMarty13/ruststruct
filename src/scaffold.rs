//! The scaffolding engine: plan a project layout, then materialise it.
//!
//! A run is described entirely by [`ScaffoldOptions`], which is the single
//! source of truth. [`scaffold`] either prints that plan ([`ScaffoldOptions::dry_run`])
//! or applies it — and if any step fails after the root directory has been
//! created, an internal rollback guard removes it again, so a failed run leaves
//! nothing behind.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::config::UserConfig;
use crate::error::{Error, Result};

/// Directories created in every new project.
pub const DEFAULT_DIRS: [&str; 4] = ["src", "tests", "benches", "examples"];

const GITIGNORE: &str = "/target\n**/*.rs.bk\n";

const MAIN_RS: &str = r#"fn main() {
    println!("{}", {{crate}}::greet("{{name}}"));
}
"#;

const LIB_RS: &str = r#"//! Library crate for `{{name}}`.

/// Returns a friendly greeting for `who`.
#[must_use]
pub fn greet(who: &str) -> String {
    format!("Hello, {who}!")
}

#[cfg(test)]
mod tests {
    use super::greet;

    #[test]
    fn greets_by_name() {
        assert_eq!(greet("world"), "Hello, world!");
    }
}
"#;

const INTEGRATION_RS: &str = r#"//! Integration tests exercise `{{name}}` through its public API only.

#[test]
fn library_is_reachable_from_integration_tests() {
    assert_eq!({{crate}}::greet("integration"), "Hello, integration!");
}
"#;

const EXAMPLE_RS: &str = r#"//! Run with `cargo run --example hello`.

fn main() {
    println!("{}", {{crate}}::greet("example"));
}
"#;

/// Everything a single scaffold run needs to know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaffoldOptions {
    /// Directory to create. Must not already exist.
    pub root: PathBuf,
    /// Cargo package name. Defaults to the last component of [`Self::root`].
    pub name: Option<String>,
    /// Run `git init` once the project is in place.
    pub git: bool,
    /// Print the plan instead of writing anything.
    pub dry_run: bool,
    /// Directories to create, relative to [`Self::root`].
    pub dirs: Vec<String>,
    /// Extra files to write, relative to [`Self::root`].
    ///
    /// Entries here override the built-in templates of the same path.
    pub files: BTreeMap<String, String>,
}

impl ScaffoldOptions {
    /// Starts from the built-in defaults for `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            name: None,
            git: false,
            dry_run: false,
            dirs: DEFAULT_DIRS.iter().map(|d| (*d).to_string()).collect(),
            files: BTreeMap::new(),
        }
    }

    /// Layers a user config on top of the defaults.
    #[must_use]
    pub fn with_config(mut self, config: UserConfig) -> Self {
        self.dirs.extend(config.dirs);
        self.files.extend(config.files);
        self
    }

    /// Resolves the package name, deriving it from the path when absent.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnnamedRoot`] when no name was given and the path has
    /// no usable final component (for example `.` or `..`).
    pub fn package_name(&self) -> Result<String> {
        if let Some(name) = &self.name {
            return Ok(name.clone());
        }
        self.root
            .file_name()
            .and_then(|n| n.to_str())
            .map(ToString::to_string)
            .ok_or_else(|| Error::UnnamedRoot(self.root.clone()))
    }
}

/// Removes a directory tree on drop, unless it has been disarmed.
///
/// This is the Rust counterpart of gostruct's `defer` + `failed` flag, and it
/// is strictly stronger: it also fires when a `?` returns early or the thread
/// panics, and there is no boolean left to forget to set.
struct RollbackGuard<'a> {
    root: &'a Path,
    armed: bool,
}

impl<'a> RollbackGuard<'a> {
    fn arm(root: &'a Path) -> Self {
        Self { root, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for RollbackGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            // Nothing useful to do if cleanup itself fails; the original error
            // is what the user needs to see.
            let _ = std::fs::remove_dir_all(self.root);
        }
    }
}

/// Rejects anything that is not a plain relative path staying inside the root.
///
/// Absolute paths, `..`, and even a leading `./` are refused, so a hostile or
/// careless `~/.ruststruct.json` cannot write outside the project.
fn is_safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

/// Built-in files for a project named `name`.
fn template_files(name: &str) -> BTreeMap<String, String> {
    let crate_name = name.replace('-', "_");
    let render = |template: &str| {
        template
            .replace("{{crate}}", &crate_name)
            .replace("{{name}}", name)
    };

    [
        (".gitignore", GITIGNORE.to_string()),
        ("src/main.rs", render(MAIN_RS)),
        ("src/lib.rs", render(LIB_RS)),
        ("tests/integration.rs", render(INTEGRATION_RS)),
        ("examples/hello.rs", render(EXAMPLE_RS)),
    ]
    .into_iter()
    .map(|(path, body)| (path.to_string(), body))
    .collect()
}

/// Creates the project described by `opts`, writing progress to `out`.
///
/// On [`ScaffoldOptions::dry_run`] nothing touches the filesystem: the plan is
/// written to `out` and the function returns. Otherwise the root is created,
/// populated, and handed to `cargo init` — and removed again if any step fails.
///
/// # Errors
///
/// Returns [`Error::RootExists`] if the target is already there,
/// [`Error::UnsafePath`] for a config entry escaping the root,
/// [`Error::CreateDir`] / [`Error::WriteFile`] on I/O failure, and
/// [`Error::CommandSpawn`] / [`Error::CommandFailed`] if `cargo` or `git`
/// cannot be run or reports an error.
pub fn scaffold(opts: &ScaffoldOptions, out: &mut impl Write) -> Result<()> {
    // 1. Never touch an existing directory.
    if opts.root.exists() {
        return Err(Error::RootExists(opts.root.clone()));
    }

    // 2. Resolve the package name.
    let name = opts.package_name()?;

    // 3. Templates first, user overrides last.
    let mut files = template_files(&name);
    files.extend(opts.files.clone());

    // 4. Explicit directories, plus every parent implied by a file path.
    let mut dirs: BTreeSet<PathBuf> = opts.dirs.iter().map(PathBuf::from).collect();
    for path in files.keys().map(Path::new) {
        if let Some(parent) = path.parent()
            && parent != Path::new("")
        {
            dirs.insert(parent.to_path_buf());
        }
    }

    // 5. Nothing from the config may escape the root.
    for path in dirs
        .iter()
        .map(PathBuf::as_path)
        .chain(files.keys().map(Path::new))
    {
        if !is_safe_relative(path) {
            return Err(Error::UnsafePath(path.to_path_buf()));
        }
    }

    // 6. Git cannot track an empty directory, so a scaffolded `benches/` would
    //    vanish on the first commit. Give any directory that would otherwise be
    //    empty a `.gitkeep`.
    //
    //    `Path::starts_with` matches whole components, so a file `srcgen/x.rs`
    //    does not count as content for a directory `src`.
    let placeholders: Vec<String> = dirs
        .iter()
        .filter(|dir| {
            !files
                .keys()
                .any(|file| Path::new(file).starts_with(dir.as_path()))
        })
        .map(|dir| dir.join(".gitkeep").to_string_lossy().into_owned())
        .collect();
    for path in placeholders {
        files.insert(path, String::new());
    }

    let init_args = ["init", "--name", name.as_str(), "--vcs", "none"];

    // 7. Dry run: describe, do not act. BTree ordering makes this reproducible.
    if opts.dry_run {
        writeln!(out, "[dry-run] project root : {}", opts.root.display())?;
        writeln!(out, "[dry-run] package name : {name}")?;
        writeln!(out, "[dry-run] directories  :")?;
        for dir in &dirs {
            writeln!(out, "  {}/", opts.root.join(dir).display())?;
        }
        writeln!(out, "[dry-run] files        :")?;
        for path in files.keys() {
            writeln!(out, "  {}", opts.root.join(path).display())?;
        }
        writeln!(
            out,
            "[dry-run] would run    : cargo {}",
            init_args.join(" ")
        )?;
        if opts.git {
            writeln!(out, "[dry-run] would run    : git init")?;
        }
        return Ok(());
    }

    // 8. From here on, any failure must leave the filesystem as it was found.
    std::fs::create_dir_all(&opts.root).map_err(|source| Error::CreateDir {
        path: opts.root.clone(),
        source,
    })?;
    let mut rollback = RollbackGuard::arm(&opts.root);

    // 9. Directories.
    for dir in &dirs {
        let full = opts.root.join(dir);
        std::fs::create_dir_all(&full).map_err(|source| Error::CreateDir {
            path: full.clone(),
            source,
        })?;
    }

    // 10. Files.
    for (path, body) in &files {
        let full = opts.root.join(path);
        std::fs::write(&full, body).map_err(|source| Error::WriteFile {
            path: full.clone(),
            source,
        })?;
    }

    // 11. Let cargo write Cargo.toml; it picks up the targets we just laid out.
    run_command(&opts.root, "cargo", &init_args)?;

    // 12. Optional git repository.
    if opts.git {
        run_command(&opts.root, "git", &["init"])?;
    }

    rollback.disarm();
    Ok(())
}

/// Runs `program` inside `dir`, folding a non-zero exit into an [`Error`].
fn run_command(dir: &Path, program: &str, args: &[&str]) -> Result<()> {
    let command = format!("{program} {}", args.join(" "));

    let output = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|source| Error::CommandSpawn {
            command: command.clone(),
            source,
        })?;

    if !output.status.success() {
        let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
        return Err(Error::CommandFailed {
            command,
            status: output.status.to_string(),
            output: combined.trim().to_string(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::TempDir;

    /// A path inside a fresh temp dir that does *not* exist yet.
    fn target(dir: &TempDir, name: &str) -> PathBuf {
        dir.path().join(name)
    }

    fn run(opts: &ScaffoldOptions) -> Result<String> {
        let mut out = Vec::new();
        scaffold(opts, &mut out)?;
        Ok(String::from_utf8(out).expect("output is utf-8"))
    }

    #[test]
    fn creates_the_default_layout() {
        let tmp = TempDir::new().unwrap();
        let root = target(&tmp, "myapp");

        run(&ScaffoldOptions::new(&root)).unwrap();

        for dir in DEFAULT_DIRS {
            assert!(root.join(dir).is_dir(), "missing directory {dir}");
        }
        for file in [
            ".gitignore",
            "src/main.rs",
            "src/lib.rs",
            "tests/integration.rs",
            "examples/hello.rs",
            "Cargo.toml",
        ] {
            assert!(root.join(file).is_file(), "missing file {file}");
        }

        let main_rs = std::fs::read_to_string(root.join("src/main.rs")).unwrap();
        assert!(main_rs.contains("myapp::greet"), "{main_rs}");
    }

    #[test]
    fn generated_project_compiles_and_passes_its_own_tests() {
        let tmp = TempDir::new().unwrap();
        let root = target(&tmp, "generated");

        run(&ScaffoldOptions::new(&root)).unwrap();

        // The whole point of a scaffolder: what it emits must build.
        run_command(&root, "cargo", &["test", "--quiet"]).expect("generated project must pass");
    }

    #[test]
    fn otherwise_empty_directories_get_a_gitkeep() {
        let tmp = TempDir::new().unwrap();
        let root = target(&tmp, "keepapp");

        run(&ScaffoldOptions::new(&root)).unwrap();

        // `benches/` is the one default directory with no template file in it.
        assert!(root.join("benches/.gitkeep").is_file());

        // Directories that already hold something must not get one.
        for dir in ["src", "tests", "examples"] {
            assert!(
                !root.join(dir).join(".gitkeep").exists(),
                "{dir}/ has content and should not be padded"
            );
        }
    }

    #[test]
    fn gitkeep_matches_whole_path_components() {
        let tmp = TempDir::new().unwrap();
        let root = target(&tmp, "componentapp");

        // `srcgen/` shares a textual prefix with `src`, but is a different
        // directory: it is empty and must still get its own placeholder.
        let config = UserConfig {
            dirs: vec!["srcgen".to_string()],
            files: BTreeMap::new(),
        };
        run(&ScaffoldOptions::new(&root).with_config(config)).unwrap();

        assert!(root.join("srcgen/.gitkeep").is_file());
        assert!(!root.join("src/.gitkeep").exists());
    }

    #[test]
    fn every_directory_survives_a_first_commit() {
        let tmp = TempDir::new().unwrap();
        let root = target(&tmp, "committed");

        let mut opts = ScaffoldOptions::new(&root);
        opts.git = true;
        run(&opts).unwrap();

        run_command(&root, "git", &["add", "-A"]).unwrap();

        // Identity is passed inline so the test does not depend on the machine's
        // global git config, and signing is off so it never blocks on a key.
        run_command(
            &root,
            "git",
            &[
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "initial",
            ],
        )
        .unwrap();

        let output = Command::new("git")
            .arg("ls-files")
            .current_dir(&root)
            .output()
            .unwrap();
        let tracked: Vec<&str> = std::str::from_utf8(&output.stdout)
            .unwrap()
            .lines()
            .collect();

        for dir in DEFAULT_DIRS {
            assert!(
                tracked.iter().any(|f| Path::new(f).starts_with(dir)),
                "{dir}/ vanished on the first commit; tracked: {tracked:?}"
            );
        }
    }

    #[test]
    fn hyphenated_names_become_underscored_crate_paths() {
        let tmp = TempDir::new().unwrap();
        let root = target(&tmp, "my-cool-app");

        run(&ScaffoldOptions::new(&root)).unwrap();

        let main_rs = std::fs::read_to_string(root.join("src/main.rs")).unwrap();
        assert!(main_rs.contains("my_cool_app::greet"), "{main_rs}");
        let cargo_toml = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(cargo_toml.contains("my-cool-app"), "{cargo_toml}");
    }

    #[test]
    fn custom_name_overrides_the_directory_name() {
        let tmp = TempDir::new().unwrap();
        let root = target(&tmp, "myapp");

        let mut opts = ScaffoldOptions::new(&root);
        opts.name = Some("acme-tool".to_string());
        run(&opts).unwrap();

        let cargo_toml = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(cargo_toml.contains("acme-tool"), "{cargo_toml}");
    }

    #[test]
    fn refuses_an_existing_directory_and_leaves_it_alone() {
        let tmp = TempDir::new().unwrap();
        let root = target(&tmp, "existing");
        std::fs::create_dir_all(&root).unwrap();
        let sentinel = root.join("sentinel.txt");
        std::fs::write(&sentinel, "original").unwrap();

        let err = run(&ScaffoldOptions::new(&root)).unwrap_err();

        assert!(matches!(err, Error::RootExists(_)), "got {err:?}");
        assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "original");
    }

    #[test]
    fn dry_run_writes_a_plan_and_nothing_else() {
        let tmp = TempDir::new().unwrap();
        let root = target(&tmp, "dryapp");

        let mut opts = ScaffoldOptions::new(&root);
        opts.dry_run = true;
        opts.git = true;
        let plan = run(&opts).unwrap();

        assert!(!root.exists(), "dry run created {}", root.display());
        assert!(plan.contains("package name : dryapp"), "{plan}");
        assert!(plan.contains("cargo init --name dryapp"), "{plan}");
        assert!(plan.contains("git init"), "{plan}");
    }

    #[test]
    fn dry_run_output_is_deterministic() {
        let tmp = TempDir::new().unwrap();
        let mut opts = ScaffoldOptions::new(target(&tmp, "stable"));
        opts.dry_run = true;

        let first = run(&opts).unwrap();
        for _ in 0..10 {
            assert_eq!(run(&opts).unwrap(), first, "plan ordering is not stable");
        }
    }

    #[test]
    fn rolls_back_everything_when_cargo_init_fails() {
        let tmp = TempDir::new().unwrap();
        let root = target(&tmp, "rollbackapp");

        let mut opts = ScaffoldOptions::new(&root);
        opts.name = Some("!invalid!name!".to_string());
        let err = run(&opts).unwrap_err();

        assert!(matches!(err, Error::CommandFailed { .. }), "got {err:?}");
        assert!(!root.exists(), "rollback left {} behind", root.display());
    }

    #[test]
    fn config_adds_directories_and_files() {
        let tmp = TempDir::new().unwrap();
        let root = target(&tmp, "cfgapp");

        let config = UserConfig {
            dirs: vec!["scripts".to_string()],
            files: BTreeMap::from([("rustfmt.toml".to_string(), "edition = \"2024\"\n".into())]),
        };
        run(&ScaffoldOptions::new(&root).with_config(config)).unwrap();

        assert!(root.join("scripts").is_dir());
        assert!(root.join("rustfmt.toml").is_file());
    }

    #[test]
    fn config_can_override_a_template_file() {
        let tmp = TempDir::new().unwrap();
        let root = target(&tmp, "overrideapp");

        let config = UserConfig {
            dirs: Vec::new(),
            files: BTreeMap::from([(".gitignore".to_string(), "custom\n".to_string())]),
        };
        run(&ScaffoldOptions::new(&root).with_config(config)).unwrap();

        assert_eq!(
            std::fs::read_to_string(root.join(".gitignore")).unwrap(),
            "custom\n"
        );
    }

    #[test]
    fn nested_config_paths_create_their_parents() {
        let tmp = TempDir::new().unwrap();
        let root = target(&tmp, "nested");

        let config = UserConfig {
            dirs: Vec::new(),
            files: BTreeMap::from([(
                ".github/workflows/ci.yml".to_string(),
                "name: ci\n".to_string(),
            )]),
        };
        run(&ScaffoldOptions::new(&root).with_config(config)).unwrap();

        assert!(root.join(".github/workflows").is_dir());
        assert!(root.join(".github/workflows/ci.yml").is_file());
    }

    #[test]
    fn rejects_config_paths_escaping_the_root() {
        let tmp = TempDir::new().unwrap();

        for hostile in ["../escape.txt", "/etc/passwd", "a/../../b.txt"] {
            let root = target(&tmp, "guarded");
            let config = UserConfig {
                dirs: Vec::new(),
                files: BTreeMap::from([(hostile.to_string(), "pwned".to_string())]),
            };

            let err = run(&ScaffoldOptions::new(&root).with_config(config)).unwrap_err();
            assert!(matches!(err, Error::UnsafePath(_)), "{hostile}: {err:?}");
            assert!(!root.exists(), "{hostile} created the root anyway");
        }
    }

    #[test]
    fn rejects_hostile_directories_too() {
        let tmp = TempDir::new().unwrap();
        let root = target(&tmp, "guarded-dirs");

        let config = UserConfig {
            dirs: vec!["../oops".to_string()],
            files: BTreeMap::new(),
        };

        let err = run(&ScaffoldOptions::new(&root).with_config(config)).unwrap_err();
        assert!(matches!(err, Error::UnsafePath(_)), "got {err:?}");
    }

    #[test]
    fn safe_relative_accepts_plain_paths_only() {
        assert!(is_safe_relative(Path::new("src/main.rs")));
        assert!(is_safe_relative(Path::new("a/b/c")));

        assert!(!is_safe_relative(Path::new("")));
        assert!(!is_safe_relative(Path::new("./src")));
        assert!(!is_safe_relative(Path::new("../src")));
        assert!(!is_safe_relative(Path::new("/abs")));
    }

    #[test]
    fn package_name_falls_back_to_the_directory_name() {
        assert_eq!(
            ScaffoldOptions::new("/tmp/some/myapp")
                .package_name()
                .unwrap(),
            "myapp"
        );

        let err = ScaffoldOptions::new("..").package_name().unwrap_err();
        assert!(matches!(err, Error::UnnamedRoot(_)), "got {err:?}");
    }

    #[test]
    fn git_flag_initialises_a_repository() {
        let tmp = TempDir::new().unwrap();
        let root = target(&tmp, "gitapp");

        let mut opts = ScaffoldOptions::new(&root);
        opts.git = true;
        run(&opts).unwrap();

        assert!(root.join(".git").exists(), "git init did not run");
    }
}
