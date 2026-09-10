use super::*;
use muxy_protocol::MetadataEvent;

fn wait_links(
    connection: &Connection,
    attachment: &mut Attachment,
    expected: Option<&str>,
) -> TestResult {
    let deadline = Instant::now() + TIMEOUT;
    let mut received_links = false;
    loop {
        match connection
            .events
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))?
        {
            ClientEvent::Metadata {
                channel,
                event: MetadataEvent::Links { seq, rows },
            } if channel == attachment.channel => {
                received_links = match expected {
                    Some(uri) => rows
                        .iter()
                        .flat_map(|r| &r.spans)
                        .any(|span| span.uri == uri),
                    None => rows.is_empty(),
                };
                attachment.grid.links.replace(seq, rows);
            }
            ClientEvent::Frame { channel, frame } => {
                if channel == attachment.channel {
                    attachment.grid.apply(&frame);
                }
                connection.client.ack(channel, frame.seq)?;
            }
            ClientEvent::Metadata { .. } => {}
            event => return Err(format!("unexpected hyperlink event: {event:?}").into()),
        }
        if received_links
            && match expected {
                Some(uri) => (0..attachment.grid.size.rows).any(|row| {
                    attachment
                        .grid
                        .links
                        .row(row, attachment.grid.size)
                        .iter()
                        .any(|span| span.uri == uri)
                }),
                None => (0..attachment.grid.size.rows).all(|row| {
                    attachment
                        .grid
                        .links
                        .row(row, attachment.grid.size)
                        .is_empty()
                }),
            }
        {
            return Ok(());
        }
    }
}

#[test]
fn links_cross_the_live_wire_clear_and_return_on_reattach() -> TestResult {
    let fixture = Fixture::new()?;
    let connection = fixture.connect()?;
    let session = fixture.create(&connection.client)?;
    let mut attached = connection.client.attach(session.id, SIZE)?;
    wait_links(&connection, &mut attached, None)?;
    connection.client.send_input(
        attached.channel,
        b"printf '\\033[2J\\033[H\\033]8;;https://example.com\\033\\\\link\\033]8;;\\033\\\\\\n'\n",
    )?;
    wait_links(&connection, &mut attached, Some("https://example.com"))?;
    connection.client.detach(attached.channel)?;
    let reconnected = fixture.connect()?;
    let mut attached = reconnected.client.attach(session.id, SIZE)?;
    wait_links(&reconnected, &mut attached, Some("https://example.com"))?;
    reconnected
        .client
        .send_input(attached.channel, b"printf '\\033[2J\\033[H'\n")?;
    wait_links(&reconnected, &mut attached, None)?;
    reconnected.client.end_session(session.id)?;
    Ok(())
}
