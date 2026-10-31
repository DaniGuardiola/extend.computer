//! Replays native/shared/fixtures/display-helper.json and display-packets.json.
use super::{display::*, input::*, packet::*};
use crate::display::input::{Binding, Command};
use anyhow::Result;
use serde_json::Value;

fn fixture(name: &str) -> Value {
    let text = match name {
        "display-helper" => include_str!("../../../native/shared/fixtures/display-helper.json"),
        _ => include_str!("../../../native/shared/fixtures/display-packets.json"),
    };
    serde_json::from_str(text).unwrap()
}

/// Parses one vector as the app does and returns what Rust would write back,
/// for the channels Rust writes.
fn read(channel: &str, json: &str) -> Result<Option<String>> {
    let bytes = json.as_bytes();
    let text = |bytes: Vec<u8>| Some(String::from_utf8(bytes).unwrap());
    Ok(match channel {
        "startup" => {
            let startup: Startup = serde_json::from_slice(bytes)?;
            if let Some(target) = &startup.target {
                target.validate()?;
            }
            text(startup.to_bytes()?)
        }
        "control" => {
            let message: HelperMessage = serde_json::from_slice(bytes)?;
            if let HelperMessage::Target { target } = &message {
                target.validate()?;
            }
            text(message.to_bytes()?)
        }
        "cursor" => {
            CursorPacket::parse(bytes)?;
            None
        }
        "viewer_input" => {
            NativeEvent::parse(bytes)?;
            None
        }
        "screen" => text(serde_json::from_slice::<Screen>(bytes)?.to_bytes()?),
        "source_stdin" => {
            let command = SourceCommand::parse_line(json).ok_or(anyhow::anyhow!("Unread"))?;
            let line = command.to_line()?;
            text(line[..line.len() - 1].to_vec())
        }
        "viewer_control" => text(serde_json::from_slice::<ViewerCommand>(bytes)?.to_bytes()?),
        "status" => Some(Status::parse(json).to_string()),
        "control_ready" => Some(ControlReady::parse(json)?.to_string()),
        "inject_display_target" => Some(serde_json::from_str::<DisplayTarget>(json)?.to_arg()?),
        "inject_display_ready" => {
            DisplayReady::parse_line(json)?;
            None
        }
        "inject_display_applied" => {
            Applied::parse_line(json)?;
            None
        }
        _ => panic!("Unknown channel {channel}"),
    })
}

#[test]
fn messages_parse_and_rust_written_ones_keep_their_bytes() {
    let fixture = fixture("display-helper");
    for vector in fixture["messages"].as_array().unwrap() {
        let (name, channel) = (&vector["name"], vector["channel"].as_str().unwrap());
        let json = vector["json"].as_str().unwrap();
        let written = read(channel, json).unwrap_or_else(|error| panic!("{name}: {error:#}"));
        if vector["from"] != "macos" {
            assert_eq!(written.as_deref(), Some(json), "{name}");
        }
        if channel == "control" {
            let ignored =
                serde_json::from_str::<HelperMessage>(json).unwrap() == HelperMessage::Unknown;
            assert_eq!(ignored, vector["ignored"] == true, "{name}");
        }
    }
    for vector in fixture["rejected"].as_array().unwrap() {
        let channel = vector["channel"].as_str().unwrap();
        let json = vector["json"].as_str().unwrap();
        assert!(read(channel, json).is_err(), "{} accepted", vector["why"]);
    }
}

#[test]
fn inject_display_lines_match_the_binding_and_target() {
    let fixture = fixture("display-helper");
    let section = &fixture["inject_display"];
    let binding: Binding = serde_json::from_value(section["binding"].clone()).unwrap();
    let target: CaptureTarget = serde_json::from_value(section["target"].clone()).unwrap();
    target.validate().unwrap();
    let expected = DisplayTarget::new(&binding, &target);
    let messages = fixture["messages"].as_array().unwrap();
    let lines = |channel: &'static str| {
        messages
            .iter()
            .filter(move |vector| vector["channel"] == channel)
            .map(|vector| vector["json"].as_str().unwrap())
    };
    for line in lines("inject_display_target") {
        assert_eq!(expected.to_arg().unwrap(), line);
    }
    for line in lines("inject_display_ready") {
        assert_eq!(DisplayReady::parse_line(line).unwrap().target, expected);
    }
    for line in lines("inject_display_applied") {
        assert_eq!(
            Applied::parse_line(line).unwrap(),
            Applied::new(&binding, 4)
        );
    }
    for envelope in section["envelopes"].as_array().unwrap() {
        let command: Command = serde_json::from_value(envelope["command"].clone()).unwrap();
        let sequence = envelope["sequence"].as_u64().unwrap();
        let bytes = target
            .event(&binding, sequence, &command)
            .unwrap()
            .to_bytes()
            .unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), envelope["line"]);
    }
}

#[test]
fn packets_frame_and_decode_as_recorded() {
    let hex = |text: &str| {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect::<Vec<_>>()
    };
    let fixture = fixture("display-packets");
    for vector in fixture["packets"].as_array().unwrap() {
        let name = &vector["name"];
        let bytes = hex(vector["hex"].as_str().unwrap());
        let read = read_packet(&mut bytes.as_slice()).and_then(|packet| {
            let (kind, payload) = packet.unwrap();
            if kind == Kind::Frame {
                frame::decode(&payload)?;
            }
            Ok((kind, payload))
        });
        if vector["valid"] == false {
            assert!(read.is_err(), "{name} accepted");
            continue;
        }
        let (kind, payload) = read.unwrap_or_else(|error| panic!("{name}: {error:#}"));
        assert_eq!(kind as u64, vector["kind"].as_u64().unwrap(), "{name}");
        let mut written = Vec::new();
        write_packet(&mut written, kind, &payload).unwrap();
        assert_eq!(written, bytes, "{name}");
        if let Some(expected) = vector.get("frame") {
            let frame = frame::decode(&payload).unwrap();
            assert_eq!(frame.timestamp, expected["timestamp"].as_u64().unwrap());
            assert_eq!(Value::Bool(frame.keyframe), expected["keyframe"]);
            assert_eq!(frame.bytes, hex(expected["annex_b"].as_str().unwrap()));
            assert_eq!(frame::encode(&frame), payload);
        }
    }
}
