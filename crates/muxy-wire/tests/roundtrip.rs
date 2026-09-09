use std::io::{self, Read, Write};

use muxy_protocol::{CONTROL, ChannelId, ChannelKind, Message, MetadataEvent, V1};
use muxy_wire::{
    Decoder, Encoder, HEADER_LEN, Header, MAX_FRAME, MessageKind, WireError, decode, encode,
};

#[test]
fn all_samples_round_trip_in_one_stream() -> Result<(), WireError> {
    let samples = Message::samples();
    let mut bytes = Vec::new();
    let mut encoder = Encoder::new(&mut bytes);
    for message in &samples {
        encoder.send(channel(message), message)?;
    }
    let mut decoder = Decoder::new(bytes.as_slice());
    for message in samples {
        assert_eq!(decoder.next()?, (channel(&message), message));
    }
    assert!(matches!(decoder.next(), Err(WireError::Closed)));
    Ok(())
}

#[test]
fn header_bytes_are_little_endian_and_length_excludes_the_prefix() -> Result<(), WireError> {
    let header = Header::new(0x10203, ChannelId(0x1234_5678), MessageKind::Input)?;
    assert_eq!(HEADER_LEN, 11);
    assert_eq!(
        header.to_bytes(),
        [0x0a, 0x02, 0x01, 0x00, 1, 0, 0x78, 0x56, 0x34, 0x12, 9]
    );
    assert_eq!(Header::from_bytes(header.to_bytes())?, header);
    assert_eq!(header.payload_len()?, 0x10203);
    Ok(())
}

#[test]
fn input_is_raw_including_empty_and_non_utf8_bytes() -> Result<(), WireError> {
    for payload in [vec![], vec![0, 0xff, 0x80, b'\r', 0x1b]] {
        let message = Message::Input(payload.clone());
        let mut bytes = vec![0xaa; 100];
        encode(&message, ChannelId(u32::MAX), &mut bytes)?;
        assert_eq!(&bytes[HEADER_LEN..], payload);
        assert_eq!(bytes.len(), HEADER_LEN + payload.len());
        let mut decoder = Decoder::new(bytes.as_slice());
        assert_eq!(decoder.next()?, (ChannelId(u32::MAX), message));
    }
    Ok(())
}

#[test]
fn postcard_payload_has_no_duplicate_message_discriminant() -> Result<(), WireError> {
    let mut bytes = Vec::new();
    encode(&Message::Hello { versions: vec![V1] }, CONTROL, &mut bytes)?;
    assert_eq!(&bytes[HEADER_LEN..], &[1, 1]);
    encode(&Message::VersionUnsupported, CONTROL, &mut bytes)?;
    assert_eq!(bytes.len(), HEADER_LEN);
    Ok(())
}

#[test]
fn every_incomplete_sample_and_empty_stream_are_closed() -> Result<(), WireError> {
    for message in Message::samples() {
        let mut bytes = Vec::new();
        encode(&message, channel(&message), &mut bytes)?;
        for end in 0..bytes.len() {
            let mut decoder = Decoder::new(&bytes[..end]);
            assert!(
                matches!(decoder.next(), Err(WireError::Closed)),
                "{message:?} truncated at {end}"
            );
        }
    }
    Ok(())
}

#[test]
fn kind_numbers_and_both_reserved_flags_are_checked() -> Result<(), WireError> {
    let kinds = [
        MessageKind::Hello,
        MessageKind::Request,
        MessageKind::FrameAck,
        MessageKind::HelloReply,
        MessageKind::VersionUnsupported,
        MessageKind::Reply,
        MessageKind::SessionEnded,
        MessageKind::Fatal,
        MessageKind::Input,
        MessageKind::Frame,
        MessageKind::Metadata,
        MessageKind::Mouse,
    ];
    for (number, kind) in (1_u8..).zip(kinds) {
        assert_eq!(kind as u8, number);
        assert_eq!(MessageKind::from_u8(number)?, kind);
    }
    for kind in 0..=u8::MAX {
        let header = Header {
            length: 7,
            version: 1,
            channel: 0,
            kind,
        };
        if kind & 0xc0 != 0 {
            assert!(matches!(
                Header::from_bytes(header.to_bytes()),
                Err(WireError::FlagsSet(value)) if value == kind
            ));
            assert!(matches!(decode(header, &[]), Err(WireError::FlagsSet(_))));
        } else if kind == 0 || kind > 12 {
            assert!(matches!(
                Header::from_bytes(header.to_bytes()),
                Err(WireError::UnknownKind(value)) if value == kind
            ));
            assert!(matches!(
                decode(header, &[]),
                Err(WireError::UnknownKind(_))
            ));
        }
    }
    Ok(())
}

#[test]
fn unsupported_versions_and_lengths_shorter_than_the_header_are_rejected() {
    let header = Header {
        length: 7,
        version: 1,
        channel: 0,
        kind: MessageKind::VersionUnsupported as u8,
    };
    for version in [0, muxy_protocol::SUPPORTED[0].0 + 1, u16::MAX] {
        let header = Header { version, ..header };
        assert!(matches!(
            Header::from_bytes(header.to_bytes()),
            Err(WireError::UnsupportedVersion(value)) if value == version
        ));
        assert!(matches!(
            decode(header, &[]),
            Err(WireError::UnsupportedVersion(_))
        ));
    }
    for length in 0..7 {
        let bytes = Header { length, ..header }.to_bytes();
        assert!(matches!(
            Decoder::new(bytes.as_slice()).next(),
            Err(WireError::Decode(_))
        ));
    }
}

#[test]
fn malformed_payloads_mismatched_lengths_and_trailing_bytes_are_rejected() -> Result<(), WireError>
{
    let hello = Header::new(1, CONTROL, MessageKind::Hello)?;
    assert!(matches!(decode(hello, &[0x80]), Err(WireError::Decode(_))));
    let input = Header::new(1, ChannelId(1), MessageKind::Input)?;
    for payload in [&[][..], &[1, 2][..]] {
        assert!(matches!(decode(input, payload), Err(WireError::Decode(_))));
    }
    for message in Message::samples() {
        if matches!(message, Message::Input(_)) {
            continue;
        }
        let mut bytes = Vec::new();
        encode(&message, channel(&message), &mut bytes)?;
        bytes.push(0);
        let header = Header::new(
            bytes.len() - HEADER_LEN,
            channel(&message),
            MessageKind::from(&message),
        )?;
        assert!(matches!(
            decode(header, &bytes[HEADER_LEN..]),
            Err(WireError::Decode(_))
        ));
    }
    Ok(())
}

#[test]
fn the_frame_cap_includes_the_entire_header() -> Result<(), WireError> {
    let message = Message::Input(vec![0xff; MAX_FRAME - HEADER_LEN]);
    let mut bytes = Vec::new();
    encode(&message, ChannelId(1), &mut bytes)?;
    assert_eq!(bytes.len(), MAX_FRAME);
    assert_eq!(
        Decoder::new(bytes.as_slice()).next()?,
        (ChannelId(1), message)
    );
    let mut header = Header::new(MAX_FRAME - HEADER_LEN, ChannelId(1), MessageKind::Input)?;
    header.length += 1;
    assert!(matches!(
        Header::from_bytes(header.to_bytes()),
        Err(WireError::FrameTooLarge)
    ));
    assert!(matches!(
        Header::new(usize::MAX, CONTROL, MessageKind::Hello),
        Err(WireError::FrameTooLarge)
    ));
    for message in [
        Message::Input(vec![0; MAX_FRAME - HEADER_LEN + 1]),
        Message::Metadata(MetadataEvent::Title("x".repeat(MAX_FRAME))),
    ] {
        let mut written = Vec::new();
        assert!(matches!(
            Encoder::new(&mut written).send(ChannelId(1), &message),
            Err(WireError::FrameTooLarge)
        ));
        assert!(written.is_empty());
    }
    Ok(())
}

#[test]
fn partial_reads_and_writes_preserve_frames() -> Result<(), WireError> {
    let mut writer = ShortWriter(Vec::new());
    let samples = Message::samples();
    let mut encoder = Encoder::new(&mut writer);
    for message in &samples {
        encoder.send(channel(message), message)?;
    }
    let mut reader = ShortReader(writer.0.as_slice());
    let mut decoder = Decoder::new(&mut reader);
    for message in samples {
        assert_eq!(decoder.next()?, (channel(&message), message));
    }
    Ok(())
}

#[test]
fn encoder_calls_write_all_once_per_frame() -> Result<(), WireError> {
    let mut writer = CountingWriter::default();
    let mut encoder = Encoder::new(&mut writer);
    let samples = Message::samples();
    for message in &samples {
        encoder.send(channel(message), message)?;
    }
    assert_eq!(writer.writes, samples.len());
    Ok(())
}

#[test]
fn io_errors_keep_their_source() {
    let error = Decoder::new(FailingIo).next();
    assert!(
        matches!(error, Err(WireError::Io(error)) if error.kind() == io::ErrorKind::PermissionDenied)
    );
    let error = Encoder::new(FailingIo).send(CONTROL, &Message::VersionUnsupported);
    assert!(
        matches!(error, Err(WireError::Io(error)) if error.kind() == io::ErrorKind::PermissionDenied)
    );
}

fn channel(message: &Message) -> ChannelId {
    match message.channel_kind() {
        ChannelKind::Control => CONTROL,
        ChannelKind::Session => ChannelId(1),
    }
}

struct ShortReader<'a>(&'a [u8]);

impl Read for ShortReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let length = output.len().min(1);
        self.0.read(&mut output[..length])
    }
}

struct ShortWriter(Vec<u8>);

impl Write for ShortWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(&bytes[..bytes.len().min(2)])
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct CountingWriter {
    writes: usize,
}

impl Write for CountingWriter {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("expected write_all"))
    }

    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writes += 1;
        assert!(bytes.len() >= HEADER_LEN);
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct FailingIo;

impl Read for FailingIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::ErrorKind::PermissionDenied.into())
    }
}

impl Write for FailingIo {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::ErrorKind::PermissionDenied.into())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
