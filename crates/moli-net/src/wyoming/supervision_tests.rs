//! Hostile framing contracts; only in-memory input and loopback fixtures.

use super::*;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

#[tokio::test]
async fn an_event_header_requires_a_line_terminator() {
    let input = br#"{"type":"info","data":{}}"#;
    assert!(receive(&mut &input[..], 0).await.is_err());
}

#[tokio::test]
async fn declared_lengths_must_be_unsigned_integers() {
    for field in ["data_length", "payload_length"] {
        for invalid in [json!(-1), json!(1.5), json!("0"), json!(null), json!(false)] {
            let header = json!({"type": "info", (field): invalid});
            let input = format!("{header}\n");
            assert!(
                receive(&mut input.as_bytes(), 0).await.is_err(),
                "accepted {field}={invalid}"
            );
        }
    }
}

#[tokio::test]
async fn event_data_must_be_an_object() {
    for invalid in [json!(null), json!([]), json!("text"), json!(42)] {
        let input = format!("{}\n", json!({"type":"info", "data":invalid}));
        assert!(receive(&mut input.as_bytes(), 0).await.is_err());
        let block = serde_json::to_vec(&invalid).unwrap();
        let mut input =
            format!("{}\n", json!({"type":"info", "data_length":block.len()})).into_bytes();
        input.extend(block);
        assert!(receive(&mut input.as_slice(), 0).await.is_err());
    }
}

#[tokio::test]
async fn oversized_blocks_are_rejected_before_reading_them() {
    for header in [
        json!({"type":"info", "data_length":MAX_BLOCK + 1}),
        json!({"type":"info", "payload_length":1}),
    ] {
        // The sender keeps the stream open without sending the declared block.
        let (mut writer, read) = tokio::io::duplex(256);
        writer
            .write_all(format!("{header}\n").as_bytes())
            .await
            .unwrap();
        let mut reader = BufReader::new(read);
        let result = tokio::time::timeout(Duration::from_millis(250), receive(&mut reader, 0))
            .await
            .expect("must reject the length without waiting for the block");
        assert!(result.is_err());
    }
}

#[tokio::test]
async fn synthetic_service_frames_use_json_blocks_without_audio() {
    for input in [
        &include_bytes!("../../../../integrations/wyoming/fixtures/openwakeword-info.wyoming")[..],
        &include_bytes!("../../../../integrations/wyoming/fixtures/piper-info.wyoming")[..],
        &include_bytes!("../../../../integrations/wyoming/fixtures/whisper-info.wyoming")[..],
    ] {
        let mut reader = input;
        let event = receive(&mut reader, 0).await.unwrap();
        assert_eq!(event.kind, "info");
        assert!(event.data.is_object());
        assert!(event.payload.is_empty());
        assert!(reader.is_empty(), "the frame must be consumed exactly");
    }
}

#[tokio::test]
async fn data_block_overrides_inline_data() {
    let block = br#"{"asr":[],"tts":[],"wake":[]}"#;
    let mut input = format!(
        "{}\n",
        json!({"type":"info", "data":{"tts":"overridden"}, "data_length":block.len()})
    )
    .into_bytes();
    input.extend(block);
    let event = receive(&mut input.as_slice(), 0).await.unwrap();
    assert_eq!(event.data["tts"], json!([]));
}

#[tokio::test]
async fn headers_and_event_types_are_required_and_bounded() {
    for header in [
        json!([]),
        json!(null),
        json!({}),
        json!({"type": 42}),
        json!({"type": ""}),
    ] {
        let input = format!("{header}\n");
        assert!(receive(&mut input.as_bytes(), 0).await.is_err());
    }
    let input = format!("{}\n", json!({"type": "x".repeat(129)}));
    assert!(receive(&mut input.as_bytes(), 0).await.is_err());
    let input = vec![b' '; MAX_BLOCK + 1];
    assert!(receive(&mut input.as_slice(), 0).await.is_err());
}

#[tokio::test]
async fn zero_lengths_preserve_inline_data() {
    let input =
        b"{\"type\":\"info\",\"data\":{\"tts\":[]},\"data_length\":0,\"payload_length\":0}\n";
    let event = receive(&mut &input[..], 0).await.unwrap();
    assert_eq!(event.data, json!({"tts": []}));
}

#[tokio::test]
async fn truncated_json_and_payload_blocks_are_rejected() {
    for input in [
        &b"{\"type\":\"info\",\"data_length\":10}\n{}"[..],
        &b"{\"type\":\"audio-chunk\",\"payload_length\":4}\n12"[..],
    ] {
        assert!(receive(&mut &input[..], 4).await.is_err());
    }
}

#[tokio::test]
async fn binary_length_is_rejected_before_waiting_for_json() {
    let (mut writer, read) = tokio::io::duplex(256);
    writer
        .write_all(b"{\"type\":\"info\",\"data_length\":32,\"payload_length\":1}\n")
        .await
        .unwrap();
    let result = tokio::time::timeout(
        Duration::from_millis(250),
        receive(&mut BufReader::new(read), 0),
    )
    .await
    .expect("binary payload must be refused before any block read");
    assert!(result.is_err());
}

/// All fake services require exactly one empty describe, followed by EOF.
async fn reply_once(frame: Vec<u8>, fragment: usize) -> (u16, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let (read, mut write) = tcp.into_split();
        let mut reader = BufReader::new(read);
        let request = receive(&mut reader, 0).await.unwrap();
        assert_eq!(request.kind, "describe");
        assert_eq!(request.data, json!({}));
        assert!(request.payload.is_empty());
        for part in frame.chunks(fragment) {
            if write.write_all(part).await.is_err() {
                break;
            }
            tokio::task::yield_now().await;
        }
        let mut extra = [0];
        let count = tokio::time::timeout(Duration::from_secs(2), reader.read(&mut extra))
            .await
            .expect("describe must close its connection")
            .unwrap();
        assert_eq!(count, 0, "no audio or second request may be sent");
    });
    (port, task)
}

#[tokio::test]
async fn describe_reads_each_fragmented_service_fixture_and_closes() {
    for frame in [
        &include_bytes!("../../../../integrations/wyoming/fixtures/openwakeword-info.wyoming")[..],
        &include_bytes!("../../../../integrations/wyoming/fixtures/piper-info.wyoming")[..],
        &include_bytes!("../../../../integrations/wyoming/fixtures/whisper-info.wyoming")[..],
    ] {
        let (port, task) = reply_once(frame.to_vec(), 7).await;
        let data = describe("127.0.0.1", port, Duration::from_secs(2))
            .await
            .unwrap();
        let mut cursor = frame;
        assert_eq!(data, receive(&mut cursor, 0).await.unwrap().data);
        task.await.unwrap();
    }
}

#[tokio::test]
async fn describe_accepts_inline_data_and_block_overrides() {
    let frame = b"{\"type\":\"info\",\"data\":{\"tts\":[]},\"data_length\":11}\n{\"wake\":[]}";
    let (port, task) = reply_once(frame.to_vec(), frame.len()).await;
    let data = describe("127.0.0.1", port, Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(data, json!({"tts":[], "wake":[]}));
    task.await.unwrap();
}

#[tokio::test]
async fn describe_rejects_unexpected_binary_truncated_and_hostile_replies() {
    for frame in [
        &b"{\"type\":\"audio-stop\"}\n"[..],
        &b"{\"type\":\"info\",\"payload_length\":1}\n"[..],
        &b"{\"type\":\"info\",\"data_length\":65537}\n"[..],
        &b"{\"type\":\"error\",\"data\":{\"text\":\"PRIVATE_TOKEN user@example.com\"}}\n"[..],
        &b"not JSON\n"[..],
    ] {
        let (port, task) = reply_once(frame.to_vec(), frame.len()).await;
        let error = describe("127.0.0.1", port, Duration::from_secs(2))
            .await
            .unwrap_err();
        assert_eq!(format!("{error:#}"), "Wyoming description unavailable");
        task.await.unwrap();
    }
}

#[tokio::test]
async fn describe_has_one_global_deadline_and_drops_the_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let (read, mut write) = tcp.into_split();
        let mut reader = BufReader::new(read);
        assert_eq!(receive(&mut reader, 0).await.unwrap().kind, "describe");
        // Never complete a header, despite making continuous progress.
        let slowly = async {
            loop {
                if write.write_all(b" ").await.is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        let mut extra = [0];
        tokio::select! {
            () = slowly => {}
            result = reader.read(&mut extra) => match result {
                Ok(0) => {}
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
                other => panic!("expected socket closure, got {other:?}"),
            },
        }
    });
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        describe("127.0.0.1", port, Duration::from_millis(100)),
    )
    .await
    .expect("ongoing partial reads must not reset the deadline");
    assert!(result.is_err());
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("the timed-out connection must be released")
        .unwrap();
}
