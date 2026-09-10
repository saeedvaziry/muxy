use std::os::unix::{ffi::OsStringExt, fs::PermissionsExt};

use super::*;

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("muxy-openers-{}", muxy_app_core::PaneId::new()));
        std::fs::create_dir(&root).expect("temp directory");
        Self(root)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn context() -> OpenContext {
    OpenContext {
        project: muxy_app_core::ProjectId::new(),
        pane: muxy_app_core::PaneId::new(),
        server: muxy_app_core::ServerId::local(),
        directory: "/project/sub directory".into(),
        project_directory: "/project".into(),
    }
}

fn file(path: impl Into<PathBuf>) -> FileLocation {
    FileLocation {
        path: path.into(),
        line: Some(12),
        column: Some(3),
    }
}

#[test]
fn builtins_route_without_launching_apps_and_preserve_unavailable_preferences() {
    let request = OpenRequest {
        target: Target::File(file("/tmp/a 'quoted'.rs")),
        context: context(),
    };
    for (id, args) in [
        (
            "system.finder",
            vec![OsString::from("-R"), "/tmp/a 'quoted'.rs".into()],
        ),
        (
            "system.application",
            vec![OsString::from("/tmp/a 'quoted'.rs")],
        ),
    ] {
        let settings = muxy_settings::OpenerSettings {
            file: id.into(),
            ..muxy_settings::OpenerSettings::default()
        };
        open_with(
            &request,
            &settings,
            || panic!("no discovery needed"),
            |command| {
                assert_eq!(command, &Launch::system(args.clone()));
                Ok(())
            },
        )
        .expect("open");
    }
    let settings = muxy_settings::OpenerSettings {
        file: "extension:unavailable".into(),
        project_target: Some(FINDER.into()),
        ..muxy_settings::OpenerSettings::default()
    };
    open_with(
        &request,
        &settings,
        || panic!("Finder bypasses discovery"),
        |command| {
            assert_eq!(command, &finder(Path::new("/tmp/a 'quoted'.rs")));
            Ok(())
        },
    )
    .expect("fallback");
    assert_eq!(settings.file, "extension:unavailable");
    let request = OpenRequest {
        target: Target::web_url("https://example.com/a?x=1&y=2").expect("url"),
        context: context(),
    };
    open_with(
        &request,
        &settings,
        || panic!("browser bypasses discovery"),
        |command| {
            assert_eq!(
                command,
                &Launch::system(["https://example.com/a?x=1&y=2".into()])
            );
            Ok(())
        },
    )
    .expect("browser");
}

#[test]
fn editor_selection_and_failed_launch_fall_back_without_losing_context() {
    let editors = [
        Editor {
            bundle: "first".into(),
            path: "/Applications/First.app".into(),
            rank: 0,
        },
        Editor {
            bundle: "preferred".into(),
            path: "/Applications/Preferred.app".into(),
            rank: 1,
        },
    ];
    let request = OpenRequest {
        target: Target::File(file("/tmp/file.rs")),
        context: context(),
    };
    for (preference, application) in [
        ("preferred", "/Applications/Preferred.app"),
        ("missing", "/Applications/First.app"),
    ] {
        let settings = muxy_settings::OpenerSettings {
            project_target: Some(preference.into()),
            ..muxy_settings::OpenerSettings::default()
        };
        let mut calls = 0;
        open_with(
            &request,
            &settings,
            || &editors,
            |command| {
                calls += 1;
                if calls == 1 {
                    assert_eq!(
                        command,
                        &Launch::system([
                            "-a".into(),
                            application.into(),
                            "/project".into(),
                            "/tmp/file.rs".into()
                        ])
                    );
                    Err(io::Error::other("editor failed"))
                } else {
                    assert_eq!(command, &finder(Path::new("/tmp/file.rs")));
                    Ok(())
                }
            },
        )
        .expect("Finder fallback");
        assert_eq!(calls, 2);
        assert_eq!(settings.project_target.as_deref(), Some(preference));
    }
    let mut remote = request;
    remote.context.server = muxy_app_core::ServerId::new();
    assert!(
        open_with(
            &remote,
            &muxy_settings::OpenerSettings::default(),
            || &editors,
            |_| panic!("remote file must not launch")
        )
        .is_err()
    );
    assert!(resolve("/etc/hosts", &remote.context).is_none());
    assert!(resolve("https://example.com", &remote.context).is_some());
}

#[test]
fn editor_cli_arguments_are_lossless_and_directories_are_not_line_locations() {
    let temp = Temp::new();
    let directory = temp.0.join("project space");
    std::fs::create_dir(&directory).expect("project");
    for (bundle, relative, goto) in [
        (
            "com.microsoft.VSCode",
            "Contents/Resources/app/bin/code",
            true,
        ),
        (
            "com.todesktop.230313mzl4w4u92",
            "Contents/Resources/app/bin/cursor",
            true,
        ),
        ("dev.zed.Zed", "Contents/MacOS/cli", false),
    ] {
        let editor = Editor {
            bundle: bundle.into(),
            path: temp.0.join(bundle),
            rank: 0,
        };
        let cli = editor.path.join(relative);
        std::fs::create_dir_all(cli.parent().expect("parent")).expect("app");
        std::fs::write(&cli, b"#!/bin/sh\nexit 0\n").expect("stub");
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).expect("executable");
        let location = file(PathBuf::from(OsString::from_vec(
            b"/tmp/file '\xff.rs".to_vec(),
        )));
        let command = editor.command(&location, &directory);
        let mut args = vec![directory.as_os_str().to_owned()];
        if goto {
            args.push("--goto".into());
        }
        args.push(OsString::from_vec(b"/tmp/file '\xff.rs:12:3".to_vec()));
        assert_eq!(
            command,
            Launch {
                program: cli.clone(),
                args
            }
        );
        launch(&command).expect("execute stub, not a shell command string");
        let command = editor.command(&file(&directory), &directory);
        assert_eq!(
            command,
            Launch {
                program: cli,
                args: vec![directory.as_os_str().to_owned()]
            }
        );
    }
    assert_eq!(
        finder(&directory),
        Launch::system(["-a".into(), "Finder".into(), directory.into_os_string()])
    );
}

#[test]
fn editor_discovery_is_bounded_and_ignores_unrelated_bundles() {
    let temp = Temp::new();
    for (name, bundle) in [
        ("Code", "com.microsoft.VSCode"),
        ("Other", "com.apple.Safari"),
    ] {
        let contents = temp.0.join(format!("{name}.app/Contents"));
        std::fs::create_dir_all(&contents).expect("app");
        std::fs::write(contents.join("Info.plist"), format!("<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>{bundle}</string></dict></plist>")).expect("plist");
    }
    let mut found = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    discover(&temp.0, 0, &mut 0, deadline, &mut found);
    assert!(found.is_empty());
    discover(&temp.0, 0, &mut 20, Instant::now(), &mut found);
    assert!(found.is_empty());
    discover(&temp.0, 0, &mut 20, deadline, &mut found);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].bundle, "com.microsoft.VSCode");
    assert_eq!(found[0].path, temp.0.join("Code.app"));
    assert!(editor_rank("com.jetbrains.toolbox").is_none());
    assert!(editor_rank("com.jetbrains.some-editor").is_some());
}

#[test]
fn a_timed_out_opener_is_reaped_and_the_worker_accepts_later_work() {
    let temp = Temp::new();
    let pid_file = temp.0.join("pid");
    let pid_path = pid_file.clone();
    let pool = WorkerPool::new("opener-timeout-test", 1, 4).expect("pool");
    let (send, receive) = std::sync::mpsc::channel();
    let finished = send.clone();
    pool.try_spawn(move || {
        let error = process::run(
            Command::new("/bin/sh")
                .args(["-c", "echo $$ > \"$1\"; while :; do :; done", "stub"])
                .arg(pid_path)
                .stdout(Stdio::null()),
            Duration::from_millis(150),
        )
        .expect_err("must time out");
        finished.send(Err(error.kind())).expect("result");
    })
    .expect("hang job");
    pool.try_spawn(move || {
        let command = Launch {
            program: "/usr/bin/true".into(),
            args: Vec::new(),
        };
        send.send(launch(&command).map_err(|error| error.kind()))
            .expect("result");
    })
    .expect("later job");
    assert_eq!(
        receive
            .recv_timeout(Duration::from_secs(5))
            .expect("timeout released worker"),
        Err(io::ErrorKind::TimedOut)
    );
    assert_eq!(
        receive
            .recv_timeout(Duration::from_secs(5))
            .expect("later work ran"),
        Ok(())
    );
    let pid = std::fs::read_to_string(pid_file).expect("pid");
    assert!(
        !Command::new("/bin/kill")
            .args(["-0", pid.trim()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("probe")
            .success()
    );
}
