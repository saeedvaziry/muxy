use std::env;
use std::error::Error;
use std::io::{self, Write};
use std::process::ExitCode;

use muxy_protocol::{Message, Size, validate_size};

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
            let _ = writeln!(io::stderr(), "messages: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut stdout = io::stdout().lock();
    match args.as_slice() {
        [] => {
            for message in Message::samples() {
                writeln!(stdout, "{:?}\n{message:#?}", message.channel_kind())?;
            }
        }
        [flag, value] if flag == "--size" => {
            let (cols, rows) = value.split_once('x').ok_or("size must be COLSxROWS")?;
            let size = Size {
                cols: cols.parse()?,
                rows: rows.parse()?,
            };
            match validate_size(size) {
                Ok(()) => writeln!(stdout, "ok")?,
                Err(error) => writeln!(stdout, "{error:?}")?,
            }
        }
        _ => return Err("usage: messages [--size COLSxROWS]".into()),
    }
    Ok(())
}
