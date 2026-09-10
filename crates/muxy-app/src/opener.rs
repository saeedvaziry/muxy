use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

mod process;

use muxy_app_core::opener::{FileLocation, OpenContext, OpenRequest, Registry, Target};
use muxy_core::worker::WorkerPool;

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests;

const FINDER: &str = "com.apple.finder";

pub(crate) fn submit<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> io::Result<async_channel::Receiver<T>> {
    static WORKERS: OnceLock<Result<WorkerPool, String>> = OnceLock::new();
    let pool = WORKERS
        .get_or_init(|| WorkerPool::new("resource-openers", 2, 32).map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|error| io::Error::other(error.clone()))?;
    let (send, receive) = async_channel::bounded(1);
    pool.try_spawn(move || {
        if !send.is_closed() {
            let _ = send.try_send(work());
        }
    })?;
    Ok(receive)
}

pub(crate) fn resolve(text: &str, context: &OpenContext) -> Option<Target> {
    Target::web_url(text).or_else(|| {
        if context.server != muxy_app_core::ServerId::local() {
            return None;
        }
        let home = std::env::var_os("HOME").map(PathBuf::from);
        Target::local_file(text, &context.directory, home.as_deref(), Path::exists)
    })
}

#[derive(Clone, Copy, Debug)]
enum Handler {
    Browser,
    Finder,
    System,
    Editor,
}

fn registry() -> &'static Registry<Handler> {
    static REGISTRY: OnceLock<Registry<Handler>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut registry = Registry::default();
        registry.register(
            "system.browser",
            "Default Browser",
            |r| matches!(&r.target, Target::Url(url) if Target::web_url(url).is_some()),
            Handler::Browser,
        );
        for (id, title, handler) in [
            ("system.finder", "Finder", Handler::Finder),
            ("system.application", "Default Application", Handler::System),
            ("system.editor", "Project Editor", Handler::Editor),
        ] {
            registry.register(
                id,
                title,
                |r| {
                    r.context.server == muxy_app_core::ServerId::local()
                        && matches!(r.target, Target::File(_))
                },
                handler,
            );
        }
        registry
    })
}

pub(crate) fn open(
    request: &OpenRequest,
    settings: &muxy_settings::OpenerSettings,
) -> io::Result<()> {
    open_with(request, settings, editors, launch)
}

fn open_with<'a>(
    request: &OpenRequest,
    settings: &muxy_settings::OpenerSettings,
    editors: impl FnOnce() -> &'a [Editor],
    mut launch: impl FnMut(&Launch) -> io::Result<()>,
) -> io::Result<()> {
    let (preferred, fallback) = match request.target {
        Target::Url(_) => (&settings.url, "system.browser"),
        Target::File(_) => (&settings.file, "system.editor"),
    };
    let handler = registry()
        .resolve(request, preferred, fallback)
        .ok_or_else(|| io::Error::other("No opener supports this resource"))?
        .handler;
    match (&request.target, handler) {
        (Target::Url(url), Handler::Browser) => launch(&Launch::system([OsString::from(url)])),
        (Target::File(file), Handler::Finder) => launch(&finder(&file.path)),
        (Target::File(file), Handler::System) => {
            launch(&Launch::system([file.path.clone().into_os_string()]))
        }
        (Target::File(file), Handler::Editor) => {
            let selected = settings.project_target.as_deref();
            if selected == Some(FINDER) {
                return launch(&finder(&file.path));
            }
            let editors = editors();
            let editor = selected
                .and_then(|id| editors.iter().find(|editor| editor.bundle == id))
                .or_else(|| editors.first());
            let command = editor.map_or_else(
                || finder(&file.path),
                |editor| editor.command(file, &request.context.project_directory),
            );
            launch(&command).or_else(|_| launch(&finder(&file.path)))
        }
        _ => Err(io::Error::other("Opener does not support this resource")),
    }
}

#[derive(Debug, Eq, PartialEq)]
struct Launch {
    program: PathBuf,
    args: Vec<OsString>,
}

impl Launch {
    fn system(args: impl IntoIterator<Item = OsString>) -> Self {
        Self {
            program: "/usr/bin/open".into(),
            args: args.into_iter().collect(),
        }
    }
}

fn launch(launch: &Launch) -> io::Result<()> {
    process::run(
        Command::new(&launch.program)
            .args(&launch.args)
            .stdout(Stdio::null()),
        Duration::from_secs(10),
    )
    .map(drop)
}

fn finder(path: &Path) -> Launch {
    if path.is_dir() {
        Launch::system([
            OsString::from("-a"),
            "Finder".into(),
            path.as_os_str().into(),
        ])
    } else {
        Launch::system([OsString::from("-R"), path.as_os_str().into()])
    }
}

#[derive(Debug)]
struct Editor {
    bundle: String,
    path: PathBuf,
    rank: usize,
}

impl Editor {
    fn command(&self, file: &FileLocation, directory: &Path) -> Launch {
        let cli = match self.bundle.as_str() {
            "com.microsoft.VSCode" => Some(("Contents/Resources/app/bin/code", true)),
            "com.microsoft.VSCodeInsiders" => {
                Some(("Contents/Resources/app/bin/code-insiders", true))
            }
            "com.vscodium" => Some(("Contents/Resources/app/bin/codium", true)),
            "com.todesktop.230313mzl4w4u92" => Some(("Contents/Resources/app/bin/cursor", true)),
            "com.exafunction.windsurf" => Some(("Contents/Resources/app/bin/windsurf", true)),
            "dev.zed.Zed" => Some(("Contents/MacOS/cli", false)),
            _ => None,
        };
        if let Some((relative, goto)) = cli {
            let program = self.path.join(relative);
            if executable(&program) {
                let mut args = vec![directory.as_os_str().to_owned()];
                if file.path.is_dir() {
                    if file.path != directory {
                        args.push(file.path.as_os_str().to_owned());
                    }
                } else {
                    if goto {
                        args.push("--goto".into());
                    }
                    args.push(location_argument(file));
                }
                return Launch { program, args };
            }
        }
        let mut args = vec![
            OsString::from("-a"),
            self.path.clone().into_os_string(),
            directory.as_os_str().into(),
        ];
        if file.path != directory {
            args.push(file.path.clone().into_os_string());
        }
        Launch::system(args)
    }
}

fn location_argument(file: &FileLocation) -> OsString {
    let mut target = file.path.as_os_str().to_owned();
    target.push(format!(
        ":{}:{}",
        file.line.unwrap_or(1),
        file.column.unwrap_or(1)
    ));
    target
}

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

fn editors() -> &'static [Editor] {
    static EDITORS: OnceLock<Vec<Editor>> = OnceLock::new();
    EDITORS.get_or_init(|| {
        let mut roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Applications"));
        }
        let mut found = Vec::new();
        let mut remaining = 4096;
        let deadline = Instant::now() + Duration::from_secs(5);
        for root in roots {
            discover(&root, 0, &mut remaining, deadline, &mut found);
        }
        found.sort_by(|a, b| (a.rank, &a.bundle, &a.path).cmp(&(b.rank, &b.bundle, &b.path)));
        found.dedup_by(|a, b| a.bundle == b.bundle);
        found
    })
}

fn discover(
    root: &Path,
    depth: usize,
    remaining: &mut usize,
    deadline: Instant,
    found: &mut Vec<Editor>,
) {
    let Ok(entries) = root.read_dir() else { return };
    for entry in entries.flatten() {
        if *remaining == 0 || Instant::now() >= deadline {
            return;
        }
        *remaining -= 1;
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "app") {
            let Some(bundle) =
                bundle_identifier(&path, deadline.saturating_duration_since(Instant::now()))
            else {
                continue;
            };
            if let Some(rank) = editor_rank(&bundle) {
                found.push(Editor { bundle, path, rank });
            }
        } else if depth < 3 && entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            discover(&path, depth + 1, remaining, deadline, found);
        }
    }
}

fn bundle_identifier(path: &Path, remaining: Duration) -> Option<String> {
    let mut child = process::run(
        Command::new("/usr/bin/plutil")
            .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"])
            .arg(path.join("Contents/Info.plist"))
            .stdout(Stdio::piped()),
        remaining.min(Duration::from_millis(500)),
    )
    .ok()?;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()?
        .take(1025)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 1024 {
        return None;
    }
    String::from_utf8(bytes)
        .ok()
        .map(|bundle| bundle.trim().to_owned())
}

fn editor_rank(bundle: &str) -> Option<usize> {
    const EDITORS: &[&str] = &[
        "com.microsoft.VSCode",
        "com.microsoft.VSCodeInsiders",
        "com.vscodium",
        "com.todesktop.230313mzl4w4u92",
        "dev.zed.Zed",
        "com.exafunction.windsurf",
        "com.qoder.ide",
        "com.apple.dt.Xcode",
        "com.jetbrains.PhpStorm",
        "com.jetbrains.WebStorm",
        "com.jetbrains.PyCharm",
        "com.jetbrains.IntelliJ-IDEA",
        "com.jetbrains.CLion",
        "com.jetbrains.GoLand",
        "com.jetbrains.RubyMine",
        "com.jetbrains.DataGrip",
        "com.jetbrains.Rider",
        "com.jetbrainsFleet",
        "com.panic.Nova",
        "com.sublimetext.4",
        "com.barebones.bbedit",
        "com.macromates.TextMate",
        "org.gnu.Emacs",
        "org.aquamacs.Aquamacs",
        "com.code.athas",
    ];
    EDITORS.iter().position(|id| *id == bundle).or_else(|| {
        (bundle.starts_with("com.jetbrains.") && !bundle.to_lowercase().contains("toolbox"))
            .then_some(100)
    })
}
