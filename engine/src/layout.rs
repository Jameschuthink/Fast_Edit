//! The workspace layout: the folders that hold jobs, templates, logs, songs, sfx, references and skills.
//! They are named in `aios.toml` at the workspace root, so a folder can move without an engine change
//! (dev/engine/CLI_LAYOUT.md). The engine's own files (js, looks, fonts, tools) stay in `engine_dir()`.
//! A job's own folders (review/, final/, _ai/) are not here either: they live inside the job (`job.rs`).
//!
//! The root: `--root DIR` > `AIOS_ROOT=DIR` > the first `aios.toml` found from the current folder up.

use anyhow::{anyhow, bail, Context, Result};
use std::fmt;
use std::path::{Path, PathBuf};

/// The layout file; the folder that holds it is the workspace root.
pub const FILE: &str = "aios.toml";

/// sysexits `EX_CONFIG`: a layout error exits with it, so an agent can tell it from a failed render.
pub const EX_CONFIG: i32 = 78;

/// One folder that `aios.toml` names. Every key is required: the engine holds no folder names of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Jobs,
    Templates,
    Logs,
    Songs,
    Sfx,
    Refs,
    Skills,
}

impl Key {
    pub const ALL: [Key; 7] = [Key::Jobs, Key::Templates, Key::Logs, Key::Songs, Key::Sfx, Key::Refs, Key::Skills];

    /// How many keys `aios.toml` holds: the length of every per-key array.
    const COUNT: usize = Key::ALL.len();

    /// The key's name in `aios.toml`.
    pub fn name(self) -> &'static str {
        match self {
            Key::Jobs => "jobs",
            Key::Templates => "templates",
            Key::Logs => "logs",
            Key::Songs => "songs",
            Key::Sfx => "sfx",
            Key::Refs => "refs",
            Key::Skills => "skills",
        }
    }

    fn from_name(s: &str) -> Option<Key> {
        Key::ALL.into_iter().find(|k| k.name() == s)
    }
}

/// How the workspace root was found (`aios config` prints it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    Flag,
    Env,
    /// the walk up from this folder
    Walk(PathBuf),
}

impl fmt::Display for Found {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Found::Flag => write!(f, "--root"),
            Found::Env => write!(f, "AIOS_ROOT"),
            Found::Walk(from) => write!(f, "found from {} up", from.display()),
        }
    }
}

/// One key's folder: the absolute path and the `aios.toml` line that set it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    path: PathBuf,
    line: usize,
}

/// The workspace folders, read once in `main`.
#[derive(Debug, Clone)]
pub struct Layout {
    root: PathBuf,
    found: Found,
    entries: [Entry; Key::COUNT],
}

impl Layout {
    /// Reads the workspace's `aios.toml`. `given`: the root from `--root` or `AIOS_ROOT` (clap reads both, the flag
    /// first, and says which one it was); none = the walk up from the current folder.
    ///
    /// # Errors
    /// No `aios.toml` at the given root or on the walk up; an `aios.toml` that does not parse (see [`parse`]).
    pub fn load(given: Option<(&Path, Found)>) -> Result<Layout> {
        let cwd = std::env::current_dir().context("reading the current folder")?;
        let (root, found) = locate(given, &cwd)?;
        Layout::read(root, found)
    }

    /// Reads `ROOT/aios.toml`; `found` says how `root` was chosen.
    fn read(root: PathBuf, found: Found) -> Result<Layout> {
        let file = root.join(FILE);
        let text = std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
        let lines = parse(&text).with_context(|| format!("in {}", file.display()))?;
        let entries = lines.map(|(value, line)| Entry { path: root.join(value), line });
        Ok(Layout { root, found, entries })
    }

    /// The workspace root (the folder that holds `aios.toml`).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The absolute folder of `key`.
    pub fn dir(&self, key: Key) -> &Path {
        &self.entries[key as usize].path
    }

    /// `aios config`: the root, how it was found, then one line per key (name, absolute path, source, "missing").
    pub fn report(&self) -> String {
        let mut s = format!("root       {}  ({})\n", self.root.display(), self.found);
        for k in Key::ALL {
            let e = &self.entries[k as usize];
            let missing = if e.path.is_dir() { "" } else { "  missing" };
            s += &format!("{:<10} {}  ({FILE}:{}){missing}\n", k.name(), e.path.display(), e.line);
        }
        s += &format!("{:<10} {}  (built in)\n", "engine", crate::job::engine_dir().display());
        s
    }
}

/// The root and how it was found: the given one, else the first folder from `cwd` up that holds `aios.toml`.
/// A given root must hold `aios.toml` itself (no walk from there): a wrong `AIOS_ROOT` never falls back.
fn locate(given: Option<(&Path, Found)>, cwd: &Path) -> Result<(PathBuf, Found)> {
    if let Some((dir, found)) = given {
        let dir = cwd.join(dir);
        if !dir.join(FILE).is_file() {
            bail!("no {FILE} in {} (from {found})", dir.display());
        }
        return Ok((dir, found));
    }
    cwd.ancestors()
        .find(|d| d.join(FILE).is_file())
        .map(|d| (d.to_path_buf(), Found::Walk(cwd.to_path_buf())))
        .ok_or_else(|| anyhow!("no {FILE} from {} up: run in the workspace, or set AIOS_ROOT=DIR", cwd.display()))
}

/// Parses `aios.toml`: `key = "path"` lines, `#` comments, blank lines. Returns each key's value and line, in
/// [`Key::ALL`] order.
///
/// # Errors
/// A line that is not `key = "path"`, an unknown key, a key set twice, or a key that is missing; each names the line
/// or the key, so a typo never falls back to a default.
fn parse(text: &str) -> Result<[(String, usize); Key::COUNT]> {
    let mut found: [Option<(String, usize)>; Key::COUNT] = Default::default();
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let l = raw.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let bad = || anyhow!("line {n}: expected `key = \"path\"`, got `{l}`");
        let (name, rest) = l.split_once('=').ok_or_else(bad)?;
        let (name, rest) = (name.trim(), rest.trim());
        let value = rest.strip_prefix('"').and_then(|r| r.split_once('"')).ok_or_else(bad)?;
        let (value, tail) = (value.0, value.1.trim());
        if !(tail.is_empty() || tail.starts_with('#')) || value.is_empty() {
            return Err(bad());
        }
        let names = Key::ALL.map(Key::name).join(", ");
        let key = Key::from_name(name).ok_or_else(|| anyhow!("line {n}: unknown key `{name}` (keys: {names})"))?;
        let slot = &mut found[key as usize];
        if let Some((_, first)) = slot {
            bail!("line {n}: `{name}` is set twice (first on line {first})");
        }
        *slot = Some((value.to_string(), n));
    }
    let missing: Vec<&str> = Key::ALL.into_iter().filter(|k| found[*k as usize].is_none()).map(Key::name).collect();
    if !missing.is_empty() {
        bail!("missing key(s): {}", missing.join(", "));
    }
    Ok(found.map(|v| v.expect("every key was checked above")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "# layout\njobs = \"jobs\"   # a comment\n# finals: jobs/JOB/final/\n\ntemplates = \"_templates\"\nlogs = \"logs\"\n\
        songs = \"songs\"\nsfx = \"sfx\"\nrefs = \"media/references\"\nskills = \".claude/skills\"\n";

    /// `Layout::dir` indexes `entries` by `key as usize`: the order of `Key::ALL` must be the declaration order
    #[test]
    fn key_order() {
        for (i, k) in Key::ALL.into_iter().enumerate() {
            assert_eq!(k as usize, i, "{k:?}");
        }
    }

    #[test]
    fn good_file() {
        let v = parse(GOOD).expect("good file parses");
        assert_eq!(v[Key::Jobs as usize], ("jobs".to_string(), 2));
        assert_eq!(v[Key::Templates as usize], ("_templates".to_string(), 5));
        assert_eq!(v[Key::Skills as usize], (".claude/skills".to_string(), 10));
    }

    #[test]
    fn unknown_key_names_the_line() {
        let e = parse(&GOOD.replace("templates =", "templats =")).unwrap_err().to_string();
        assert!(e.contains("line 5") && e.contains("templats"), "{e}");
    }

    /// An aios.toml from before 2026-10-28 still names `final`: it must fail loudly, not be ignored
    #[test]
    fn old_final_key_is_unknown() {
        let e = parse(&format!("{GOOD}final = \"final\"\n")).unwrap_err().to_string();
        assert!(e.contains("line 11") && e.contains("unknown key `final`"), "{e}");
    }

    #[test]
    fn bad_lines() {
        for bad in ["jobs = jobs", "jobs \"jobs\"", "jobs = \"jobs\" extra", "jobs = \"\"", "jobs = \"jobs"] {
            let e = parse(&GOOD.replace("jobs = \"jobs\"   # a comment", bad)).unwrap_err().to_string();
            assert!(e.contains("line 2"), "{bad}: {e}");
        }
    }

    #[test]
    fn missing_and_twice() {
        let e = parse(&GOOD.replace("refs = \"media/references\"\n", "")).unwrap_err().to_string();
        assert!(e.contains("missing key(s): refs"), "{e}");
        let e = parse(&format!("{GOOD}jobs = \"x\"\n")).unwrap_err().to_string();
        assert!(e.contains("line 11") && e.contains("first on line 2"), "{e}");
    }

    #[test]
    fn locate_order() {
        let tmp = std::env::temp_dir().join(format!("aios_layout_test_{}", std::process::id()));
        let (a, b) = (tmp.join("a"), tmp.join("b"));
        let deep = a.join("jobs/x");
        std::fs::create_dir_all(&deep).expect("make test folders");
        std::fs::create_dir_all(&b).expect("make test folders");
        std::fs::write(a.join(FILE), GOOD).expect("write a/aios.toml");
        std::fs::write(b.join(FILE), GOOD).expect("write b/aios.toml");

        assert_eq!(locate(None, &deep).expect("walk up"), (a.clone(), Found::Walk(deep.clone())));
        assert_eq!(locate(Some((&b, Found::Env)), &deep).expect("given root"), (b.clone(), Found::Env));
        assert!(locate(Some((&deep, Found::Env)), &deep).is_err(), "a given root without aios.toml never walks up");
        assert!(locate(None, &tmp).is_err(), "no aios.toml above");

        let l = Layout::read(a.clone(), Found::Flag).expect("read a");
        assert_eq!(l.dir(Key::Templates), a.join("_templates"));
        std::fs::remove_dir_all(&tmp).expect("clean test folders");
    }
}
