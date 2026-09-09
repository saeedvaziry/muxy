use std::io::{self, Write};

use muxy_settings::{Action, KeyChord, Settings, TerminalSettings};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let mut output = io::stdout().lock();
    match arguments.as_slice() {
        [command] if command == "show" => {
            let path = Settings::default_path()?;
            let settings = Settings::load(&path)?;
            let ghostty_path = path.with_file_name("ghostty.conf");
            let terminal = TerminalSettings::load(&ghostty_path)?;
            writeln!(
                output,
                "Settings: {}\n{}",
                path.display(),
                toml::to_string_pretty(&settings)?
            )?;
            writeln!(
                output,
                "Terminal fonts: {}\n{terminal:#?}",
                ghostty_path.display()
            )?;
            writeln!(output, "Bindings:")?;
            for action in Action::ALL {
                writeln!(
                    output,
                    "{} = {}",
                    action.name(),
                    settings
                        .keymap
                        .chord(action)
                        .map_or("unbound", KeyChord::as_str)
                )?;
            }
        }
        [command, chord] if command == "parse" => {
            let chord: KeyChord = chord.parse()?;
            writeln!(output, "{chord:?}\nCanonical: {chord}")?;
        }
        _ => return Err("usage: settings show | parse <chord>".into()),
    }
    Ok(())
}
