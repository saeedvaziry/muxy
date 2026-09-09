#![cfg(target_os = "macos")]

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use muxy_proto::extension::{ExtensionBroadcast, InvokeOutcome, InvokeRequest};
use muxy_proto::server::{
    CommandReply, ExtensionSnapshot, ExtensionSnapshotEntry, IncomingRequest, ServerConfig,
    ServerLimits, SocketServer, SubscriptionAccess,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const EXTENSION_ID: &str = "fixture-host";
const TOKEN: &str = "fixture-token";

#[test]
fn background_host_authenticates_runs_javascript_and_routes_both_directions() {
    let temporary = tempfile::tempdir().unwrap();
    let socket_path = temporary.path().join("main.sock");
    let script_path = temporary.path().join("background.js");
    std::fs::write(
        &script_path,
        r#"
console.log('fixture-started');
muxy.remote.handle('echo', (payload) => ({ echoed: payload.value }));
muxy.events.subscribe('project.switched', (payload) => {
    console.log('event:' + payload.projectID);
    const result = muxy.git.status({ project: payload.projectID });
    console.log('api:' + result.marker);
});
setTimeout(() => {
    muxy.events.emit('extension.ready', { ready: true });
}, 10);
"#,
    )
    .unwrap();
    let config = ServerConfig {
        socket_path: socket_path.clone(),
        recognized_command_heads: HashSet::from(["git.status".to_owned()]),
        no_response_command_routes: Vec::new(),
        limits: ServerLimits::default(),
        initial_extension_snapshot: ExtensionSnapshot {
            entries: BTreeMap::from([(
                EXTENSION_ID.to_owned(),
                ExtensionSnapshotEntry {
                    token: TOKEN.to_owned(),
                    granted_permissions: BTreeSet::from(["git.read".to_owned()]),
                    subscription_access: BTreeMap::from([(
                        "project.switched".to_owned(),
                        SubscriptionAccess::Allowed,
                    )]),
                    can_write_notifications: false,
                },
            )]),
        },
    };
    let (server, handle, incoming) = SocketServer::start(config).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_muxy-extension-host"))
        .arg(&script_path)
        .env("MUXY_SOCKET_PATH", &socket_path)
        .env("MUXY_EXTENSION_ID", EXTENSION_ID)
        .env("MUXY_EXTENSION_TOKEN", TOKEN)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            Instant::now() < deadline,
            "host did not emit its ready event"
        );
        match incoming.try_recv() {
            Ok(IncomingRequest::ExtensionLocalEvent(event)) => {
                assert_eq!(event.extension_id, EXTENSION_ID);
                assert_eq!(event.event.name, "extension.ready");
                assert_eq!(event.event.payload, br#"{"ready":true}"#);
                break;
            }
            Ok(IncomingRequest::AppCommand(request)) => {
                request
                    .responder
                    .respond(CommandReply::new("error:unexpected command"));
            }
            Ok(_) | Err(async_channel::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(async_channel::TryRecvError::Closed) => panic!("socket ingress closed"),
        }
    }

    handle.broadcast(ExtensionBroadcast {
        name: "project.switched".to_owned(),
        payload: BTreeMap::from([("projectID".to_owned(), "sample-project".to_owned())]),
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "host did not invoke the API");
        match incoming.try_recv() {
            Ok(IncomingRequest::AppCommand(request)) => {
                assert_eq!(request.origin.extension_id.as_deref(), Some(EXTENSION_ID));
                assert!(request.command.starts_with("git.status|"));
                request.responder.respond(CommandReply::new(STANDARD.encode(
                    serde_json::to_vec(&serde_json::json!({ "marker": "api-ok" })).unwrap(),
                )));
                break;
            }
            Ok(_) | Err(async_channel::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(async_channel::TryRecvError::Closed) => panic!("socket ingress closed"),
        }
    }

    let invocation = handle.invoke(
        EXTENSION_ID,
        InvokeRequest::new("echo", br#"{"value":"round-trip"}"#.to_vec()),
    );
    assert_eq!(
        invocation.recv_timeout(Duration::from_secs(5)).unwrap(),
        InvokeOutcome::Success(br#"{"echoed":"round-trip"}"#.to_vec())
    );

    drop(server);
    let status = wait_for_exit(&mut child, Duration::from_secs(5));
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(status.success(), "host failed: {stderr}");
    assert!(stderr.contains("[log] fixture-started"), "{stderr}");
    assert!(stderr.contains("[log] event:sample-project"), "{stderr}");
    assert!(stderr.contains("[log] api:api-ok"), "{stderr}");
}

#[test]
fn background_host_rejects_an_invalid_token() {
    let temporary = tempfile::tempdir().unwrap();
    let socket_path = temporary.path().join("main.sock");
    let script_path = temporary.path().join("background.js");
    std::fs::write(&script_path, "console.log('must-not-run');").unwrap();
    let (server, _, _) = SocketServer::start(ServerConfig {
        socket_path: socket_path.clone(),
        recognized_command_heads: HashSet::new(),
        no_response_command_routes: Vec::new(),
        limits: ServerLimits::default(),
        initial_extension_snapshot: ExtensionSnapshot {
            entries: BTreeMap::from([(
                EXTENSION_ID.to_owned(),
                ExtensionSnapshotEntry {
                    token: TOKEN.to_owned(),
                    ..Default::default()
                },
            )]),
        },
    })
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_muxy-extension-host"))
        .arg(&script_path)
        .env("MUXY_SOCKET_PATH", &socket_path)
        .env("MUXY_EXTENSION_ID", EXTENSION_ID)
        .env("MUXY_EXTENSION_TOKEN", "wrong-token")
        .output()
        .unwrap();
    drop(server);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("error:invalid extension token"), "{stderr}");
    assert!(!stderr.contains("must-not-run"), "{stderr}");
}

#[test]
fn background_host_reports_script_syntax_errors() {
    let temporary = tempfile::tempdir().unwrap();
    let socket_path = temporary.path().join("main.sock");
    let script_path = temporary.path().join("background.js");
    std::fs::write(&script_path, "const = ;").unwrap();
    let (server, _, _) = SocketServer::start(ServerConfig {
        socket_path: socket_path.clone(),
        recognized_command_heads: HashSet::new(),
        no_response_command_routes: Vec::new(),
        limits: ServerLimits::default(),
        initial_extension_snapshot: ExtensionSnapshot {
            entries: BTreeMap::from([(
                EXTENSION_ID.to_owned(),
                ExtensionSnapshotEntry {
                    token: TOKEN.to_owned(),
                    ..Default::default()
                },
            )]),
        },
    })
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_muxy-extension-host"))
        .arg(&script_path)
        .env("MUXY_SOCKET_PATH", &socket_path)
        .env("MUXY_EXTENSION_ID", EXTENSION_ID)
        .env("MUXY_EXTENSION_TOKEN", TOKEN)
        .output()
        .unwrap();
    drop(server);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("JavaScript error"), "{stderr}");
}

#[test]
fn background_host_logs_subscription_denials_and_malformed_api_replies() {
    let temporary = tempfile::tempdir().unwrap();
    let socket_path = temporary.path().join("main.sock");
    let script_path = temporary.path().join("background.js");
    std::fs::write(
        &script_path,
        r#"
        muxy.events.subscribe('agent.status', () => {});
        setTimeout(() => {
            try { muxy.git.status({}); }
            catch (error) { console.log('caught:' + error.message); }
            muxy.events.emit('extension.done', {});
        }, 10);
        "#,
    )
    .unwrap();
    let (server, _, incoming) = SocketServer::start(ServerConfig {
        socket_path: socket_path.clone(),
        recognized_command_heads: HashSet::from(["git.status".to_owned()]),
        no_response_command_routes: Vec::new(),
        limits: ServerLimits::default(),
        initial_extension_snapshot: ExtensionSnapshot {
            entries: BTreeMap::from([(
                EXTENSION_ID.to_owned(),
                ExtensionSnapshotEntry {
                    token: TOKEN.to_owned(),
                    granted_permissions: BTreeSet::from(["git.read".to_owned()]),
                    subscription_access: BTreeMap::from([(
                        "agent.status".to_owned(),
                        SubscriptionAccess::Denied("permission denied (agents:read)".to_owned()),
                    )]),
                    can_write_notifications: false,
                },
            )]),
        },
    })
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_muxy-extension-host"))
        .arg(&script_path)
        .env("MUXY_SOCKET_PATH", &socket_path)
        .env("MUXY_EXTENSION_ID", EXTENSION_ID)
        .env("MUXY_EXTENSION_TOKEN", TOKEN)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut finished = false;
    while Instant::now() < deadline && !finished {
        match incoming.try_recv() {
            Ok(IncomingRequest::AppCommand(request)) => {
                assert!(request.command.starts_with("git.status|"));
                request.responder.respond(CommandReply::new("malformed"));
            }
            Ok(IncomingRequest::ExtensionLocalEvent(event)) => {
                assert_eq!(event.event.name, "extension.done");
                finished = true;
            }
            Ok(_) | Err(async_channel::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(async_channel::TryRecvError::Closed) => break,
        }
    }
    assert!(finished, "host did not finish failure-path fixture");
    drop(server);
    let status = wait_for_exit(&mut child, Duration::from_secs(5));
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(status.success(), "host failed: {stderr}");
    assert!(
        stderr.contains("subscribe agent.status failed: error:permission denied (agents:read)"),
        "{stderr}"
    );
    assert!(
        stderr.contains("[log] caught:invalid git.status reply"),
        "{stderr}"
    );
}

fn wait_for_exit(child: &mut std::process::Child, timeout: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return child.wait().unwrap();
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
