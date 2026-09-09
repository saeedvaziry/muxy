use std::env;
use std::error::Error;
use std::io::{self, Write};
use std::process::ExitCode;

use muxy_app_core::{AppState, PaneContent, TabId, store};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if error
                .downcast_ref::<io::Error>()
                .is_some_and(|error| error.kind() == io::ErrorKind::BrokenPipe)
            {
                return ExitCode::SUCCESS;
            }
            let _ = writeln!(io::stderr(), "state: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let path = store::default_path()?;
    let mut state = store::load(&path)?;
    let home = state.home().id;
    let mut stdout = io::stdout().lock();
    match args.as_slice() {
        [command] if command == "show" => {
            writeln!(stdout, "State: {}", path.display())?;
            return show(&mut stdout, &state).map_err(Into::into);
        }
        [command] if command == "new-tab" => {
            let tab = state.open_terminal_tab(home)?;
            store::save(&path, &state)?;
            writeln!(stdout, "Opened tab {tab}")?;
        }
        [command, id] if command == "close-tab" => {
            let tab: TabId = id.parse()?;
            state.close_tab(home, tab)?;
            store::save(&path, &state)?;
            writeln!(stdout, "Closed tab {tab}")?;
        }
        [command, id] if command == "select-tab" => {
            let tab: TabId = id.parse()?;
            state.select_tab(home, tab)?;
            store::save(&path, &state)?;
            writeln!(stdout, "Selected tab {tab}")?;
        }
        [command, id, index] if command == "move-tab" => {
            let tab: TabId = id.parse()?;
            let from = state
                .home()
                .tabs
                .iter()
                .position(|candidate| candidate.id == tab)
                .ok_or_else(|| format!("tab {tab} does not belong to Home"))?;
            let to = index.parse()?;
            state.move_tab(home, from, to)?;
            store::save(&path, &state)?;
            writeln!(stdout, "Moved tab {tab} to index {to}")?;
        }
        _ => {
            return Err(
                "usage: state show | new-tab | close-tab <id> | select-tab <id> | move-tab <id> <index> (zero-based)"
                    .into(),
            );
        }
    }
    Ok(())
}

fn show(output: &mut impl Write, state: &AppState) -> io::Result<()> {
    for project in state.projects() {
        writeln!(
            output,
            "Project {} [{}] — {}",
            project.name,
            project.id,
            project.directory.display()
        )?;
        if project.tabs.is_empty() {
            writeln!(output, "  (no tabs)")?;
        }
        for tab in &project.tabs {
            let selected = if state.window().selected_tab.get(&project.id) == Some(&tab.id) {
                " (selected)"
            } else {
                ""
            };
            writeln!(
                output,
                "  Tab {}{selected} — {:?}",
                tab.id,
                tab.title(state.window().active_pane)
            )?;
            for pane in &tab.panes {
                match pane.content {
                    PaneContent::Terminal { session } => {
                        let session =
                            session.map_or_else(|| "none".into(), |id| id.get().to_string());
                        writeln!(
                            output,
                            "    Pane {} — terminal, session: {session}",
                            pane.id
                        )?;
                    }
                    PaneContent::Settings => writeln!(output, "    Pane {} — settings", pane.id)?,
                }
            }
        }
    }
    Ok(())
}
