use super::*;
use muxy_protocol::TerminalColors;

fn colors(light: bool) -> TerminalColors {
    TerminalColors {
        foreground: if light {
            [0x1e, 0x1e, 0x2e]
        } else {
            [0xc9, 0xc2, 0xd9]
        },
        background: if light {
            [0xf0, 0xf0, 0xf5]
        } else {
            [0x19, 0x17, 0x1f]
        },
        cursor: [0xc3, 0x70, 0xd3],
        ansi: [[0x12, 0x34, 0x56]; 16],
    }
}

fn replies(colors: TerminalColors) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (query, [r, g, b]) in [(10, colors.foreground), (11, colors.background)] {
        bytes.extend_from_slice(
            format!("\x1b]{query};rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}\x07").as_bytes(),
        );
    }
    bytes
}

fn query_script(expected: &[u8]) -> String {
    format!(
        "stty -echo -icanon min 1 time 0; printf '\\033]10;?\\007\\033]11;?\\007'; dd bs=1 count={} of=colors.bin 2>/dev/null; stty sane; touch colors.done\n",
        expected.len()
    )
}

fn verify_replies(fixture: &Fixture, expected: &[u8]) -> TestResult {
    let done = fixture.directory.join("colors.done");
    let deadline = Instant::now() + TIMEOUT;
    while !done.exists() {
        if Instant::now() > deadline {
            return Err("terminal color queries did not receive replies".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
    let response = fixture.directory.join("colors.bin");
    assert_eq!(fs::read(&response)?, expected);
    fs::remove_file(response)?;
    fs::remove_file(done)?;
    Ok(())
}

fn query(
    connection: &Connection,
    channel: ChannelId,
    fixture: &Fixture,
    colors: TerminalColors,
) -> TestResult {
    let expected = replies(colors);
    connection
        .client
        .send_input(channel, query_script(&expected).as_bytes())?;
    verify_replies(fixture, &expected)
}

#[test]
fn terminal_colors_are_available_to_startup_programs_before_attach() -> TestResult {
    let expected = replies(colors(false));
    let fixture = Fixture::with_startup(&query_script(&expected))?;
    let connection = fixture.connect()?;
    connection.client.set_terminal_colors(colors(false))?;
    let session = fixture.create(&connection.client)?;
    verify_replies(&fixture, &expected)?;
    let attachment = connection.client.attach(session.id, SIZE)?;
    query(&connection, attachment.channel, &fixture, colors(false))?;
    connection.client.end_session(session.id)?;
    fs::remove_file(fixture.directory.join("shell"))?;
    Ok(())
}

#[test]
fn terminal_colors_follow_theme_changes_and_colored_reattaches() -> TestResult {
    let fixture = Fixture::new()?;
    let first = fixture.connect()?;
    first.client.set_terminal_colors(colors(false))?;
    let session = fixture.create(&first.client)?;
    let attachment = first.client.attach(session.id, SIZE)?;
    query(&first, attachment.channel, &fixture, colors(false))?;

    first.client.set_terminal_colors(colors(true))?;
    query(&first, attachment.channel, &fixture, colors(true))?;
    first.client.detach(attachment.channel)?;
    drop(first);

    let second = fixture.connect()?;
    second.client.set_terminal_colors(colors(false))?;
    let attached = second.client.attach(session.id, SIZE)?;
    query(&second, attached.channel, &fixture, colors(false))?;

    let third = fixture.connect()?;
    let uncolored = third.client.attach(session.id, SIZE)?;
    query(&third, uncolored.channel, &fixture, colors(false))?;
    third.client.set_terminal_colors(colors(true))?;
    query(&second, attached.channel, &fixture, colors(true))?;
    third.client.detach(uncolored.channel)?;
    query(&second, attached.channel, &fixture, colors(true))?;
    second.client.end_session(session.id)?;
    Ok(())
}
