use std::env;
use std::error::Error;
use std::io::{self, BufRead, Read, Write};
use std::process::ExitCode;

use muxy_protocol::{CONTROL, ChannelId, ChannelKind, Message};
use muxy_wire::{HEADER_LEN, Header, WireError, decode, encode};

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
            let _ = writeln!(io::stderr(), "hexdump: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut stdout = io::stdout().lock();
    match args.as_slice() {
        [mode] if mode == "encode" => {
            let mut bytes = Vec::new();
            for message in Message::samples() {
                let channel = match message.channel_kind() {
                    ChannelKind::Control => CONTROL,
                    ChannelKind::Session => ChannelId(1),
                };
                encode(&message, channel, &mut bytes)?;
                stdout.write_all(&bytes)?;
            }
        }
        [mode] if mode == "decode" => {
            let mut stdin = io::stdin().lock();
            let mut payload = Vec::new();
            while !stdin.fill_buf()?.is_empty() {
                let mut bytes = [0; HEADER_LEN];
                stdin.read_exact(&mut bytes).map_err(WireError::from)?;
                let header = Header::from_bytes(bytes)?;
                payload.resize(header.payload_len()?, 0);
                stdin.read_exact(&mut payload).map_err(WireError::from)?;
                let (_, message) = decode(header, &payload)?;
                writeln!(
                    stdout,
                    "{} {} {} {} {message:?}",
                    header.length, header.version, header.channel, header.kind
                )?;
            }
        }
        _ => return Err("usage: hexdump encode|decode".into()),
    }
    Ok(())
}
