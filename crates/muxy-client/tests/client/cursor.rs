use super::*;
use muxy_protocol::MetadataEvent;

fn wait_blinking(connection: &Connection, channel: ChannelId, expected: bool) -> TestResult {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match connection
            .events
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))?
        {
            ClientEvent::Metadata {
                channel: received,
                event: MetadataEvent::CursorBlinking(blinking),
            } if received == channel && blinking == expected => return Ok(()),
            ClientEvent::Frame { channel, frame } => connection.client.ack(channel, frame.seq)?,
            ClientEvent::Metadata { .. } => {}
            other => return Err(format!("expected cursor metadata, got {other:?}").into()),
        }
    }
}

#[test]
fn cursor_blinking_follows_modes_and_is_resent_on_attach() -> TestResult {
    let fixture = Fixture::new()?;
    let connection = fixture.connect()?;
    let session = fixture.create(&connection.client)?;
    let attached = connection.client.attach(session.id, SIZE)?;
    wait_blinking(&connection, attached.channel, true)?;
    for (sequence, blinking) in [
        ("\\033[2 q", false),
        ("\\033[1 q", true),
        ("\\033[?12l", false),
        ("\\033[?12h", true),
        ("\\033[6 q", false),
    ] {
        connection.client.send_input(
            attached.channel,
            format!("printf '{sequence}'\n").as_bytes(),
        )?;
        wait_blinking(&connection, attached.channel, blinking)?;
    }
    connection.client.detach(attached.channel)?;
    let reconnected = fixture.connect()?;
    let attached = reconnected.client.attach(session.id, SIZE)?;
    wait_blinking(&reconnected, attached.channel, false)?;
    reconnected
        .client
        .send_input(attached.channel, b"printf '\\033[0 q'\n")?;
    wait_blinking(&reconnected, attached.channel, true)?;
    reconnected.client.end_session(session.id)?;
    Ok(())
}
