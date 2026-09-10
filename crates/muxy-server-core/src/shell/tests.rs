use super::*;

#[test]
fn disabled_integration_preserves_shell_startup() {
    let hooks = ShellIntegration {
        directory: "/tmp/muxy-hooks".into(),
    };
    let mut request = SpawnRequest {
        program: "/bin/zsh".into(),
        args: vec!["-l".into()],
        cwd: "/tmp".into(),
        env: vec![
            ("ZDOTDIR".into(), "/custom".into()),
            ("XDG_DATA_DIRS".into(), "/xdg".into()),
        ],
        size: PtySize { cols: 80, rows: 24 },
    };
    let before = request.env.clone();
    hooks.configure(&mut request, false);
    assert_eq!(&request.env[..before.len()], &before);
    assert_eq!(request.args, [OsString::from("-l")]);
    assert_eq!(
        get_env(&request, "MUXY_SHELL_INTEGRATION"),
        Some(&OsString::from("0"))
    );
}

use muxy_pty::{Pty, PtyEvent, PtySize};
use std::error::Error;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(0);
type TestResult<T = ()> = Result<T, Box<dyn Error>>;

struct Shell {
    pty: Pty,
    events: Receiver<PtyEvent>,
    directory: PathBuf,
}

impl Drop for Shell {
    fn drop(&mut self) {
        let _ = self.pty.kill();
        let _ = self.pty.wait();
        let _ = fs::remove_dir_all(&self.directory);
    }
}

impl Shell {
    fn start(shell: &str, custom: bool, enabled: bool, startup: &str) -> TestResult<Self> {
        Self::configured(shell, custom, enabled, startup, &[])
    }

    fn configured(
        shell: &str,
        custom: bool,
        enabled: bool,
        startup: &str,
        env: &[(&str, &str)],
    ) -> TestResult<Self> {
        let directory = std::env::temp_dir().join(format!(
            "muxy-shell-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory)?;
        let home = directory.join("home");
        let dotfiles = if custom {
            directory.join("custom")
        } else {
            home.clone()
        };
        fs::create_dir_all(&dotfiles)?;
        fs::create_dir_all(&home)?;
        let hooks = ShellIntegration::install(&directory.join("hooks"))?;
        let mut request = SpawnRequest {
            program: shell.into(),
            args: vec!["-l".into()],
            cwd: home.clone(),
            env: vec![
                ("HOME".into(), home.clone().into_os_string()),
                ("TERM".into(), "xterm-256color".into()),
                ("PATH".into(), "/usr/bin:/bin:/usr/sbin:/sbin".into()),
            ],
            size: PtySize {
                cols: 120,
                rows: 24,
            },
        };
        if custom {
            request
                .env
                .push(("ZDOTDIR".into(), dotfiles.clone().into_os_string()));
        }
        if shell.ends_with("zsh") {
            fs::write(
                dotfiles.join(".zshenv"),
                "typeset -g MUXY_TEST_ENV=$(( ${MUXY_TEST_ENV:-0} + 1 ))\n",
            )?;
            fs::write(
                dotfiles.join(".zprofile"),
                "typeset -g MUXY_TEST_PROFILE=loaded\n",
            )?;
            fs::write(
                dotfiles.join(".zshrc"),
                format!("PS1='muxy-test> '; unset RPS1; MUXY_TEST_RC=loaded\n{startup}\n"),
            )?;
            fs::write(
                dotfiles.join(".zlogin"),
                "typeset -g MUXY_TEST_LOGIN=loaded\n",
            )?;
        } else if shell.ends_with("fish") {
            let config = home.join(".config/fish");
            fs::create_dir_all(&config)?;
            fs::write(
                config.join("config.fish"),
                format!(
                    "set -g fish_greeting; function fish_prompt; printf 'muxy-test> '; end\n{startup}\n"
                ),
            )?;
        } else {
            fs::write(
                home.join(".bash_profile"),
                format!("PS1='muxy-test> '\n{startup}\n"),
            )?;
        }
        request.env.extend(
            env.iter()
                .map(|(key, value)| ((*key).into(), (*value).into())),
        );
        hooks.configure(&mut request, enabled);
        let pty = Pty::spawn(request)?;
        let (sender, events) = mpsc::channel();
        pty.start_reader(sender)?;
        Ok(Self {
            pty,
            events,
            directory,
        })
    }

    fn until(&mut self, expected: &[u8]) -> TestResult<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut bytes = Vec::new();
        let mut terminal = muxy_terminal::Terminal::new(
            muxy_terminal::Size {
                cols: 120,
                rows: 24,
            },
            1024 * 1024,
        )?;
        while !bytes
            .windows(expected.len())
            .any(|window| window == expected)
        {
            match self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(PtyEvent::Output(output)) => {
                    terminal.feed(&output);
                    let reply = terminal.take_pty_output();
                    if !reply.is_empty() {
                        self.pty.write(&reply)?;
                    }
                    bytes.extend(output);
                }
                other => {
                    return Err(format!(
                        "waiting for {expected:?}: {other:?}; output={:?}",
                        String::from_utf8_lossy(&bytes)
                    )
                    .into());
                }
            }
        }
        Ok(bytes)
    }
}

#[test]
fn zsh_marks_prompts_commands_and_failures_without_replacing_startup_files() -> TestResult {
    for custom in [false, true] {
        let mut shell = Shell::start("/bin/zsh", custom, true, "")?;
        let mut output = shell.until(b"\x1b]133;B\x07")?;
        shell.pty.write(b"printf 'startup:%s:%s:%s:%s\\n' $MUXY_TEST_ENV $MUXY_TEST_PROFILE $MUXY_TEST_RC $MUXY_TEST_LOGIN\n")?;
        let command = shell.until(b"\x1b]133;B\x07")?;
        assert!(String::from_utf8_lossy(&command).contains("startup:1:loaded:loaded:loaded"));
        assert!(command.windows(8).any(|bytes| bytes == b"\x1b]133;C\x07"));
        assert!(
            command
                .windows(10)
                .any(|bytes| bytes == b"\x1b]133;D;0\x07")
        );
        output.extend(command);
        shell.pty.write(b"false\n")?;
        let failed = shell.until(b"\x1b]133;B\x07")?;
        assert!(
            failed.windows(10).any(|bytes| bytes == b"\x1b]133;D;1\x07"),
            "{failed:?}"
        );
        output.extend(failed);
        let mut terminal = muxy_terminal::Terminal::new(
            muxy_terminal::Size {
                cols: 120,
                rows: 24,
            },
            1024 * 1024,
        )?;
        terminal.feed(&output);
        assert_eq!(terminal.screen_prompts()?.len(), 3);
        assert_eq!(
            fs::metadata(shell.directory.join("hooks/muxy.zsh"))?
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(shell.directory.join("hooks"))?
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    Ok(())
}

#[test]
fn disabled_zsh_has_no_marks_and_preserves_custom_zdotdir() -> TestResult {
    let mut shell = Shell::start("/bin/zsh", true, false, "")?;
    let output = shell.until(b"muxy-test> ")?;
    assert!(!output.windows(5).any(|bytes| bytes == b"]133;"));
    shell
        .pty
        .write(b"print -r -- startup:$MUXY_TEST_ENV:$MUXY_TEST_RC:$ZDOTDIR\n")?;
    let output = shell.until(b"muxy-test> ")?;
    assert!(String::from_utf8_lossy(&output).contains(&format!(
        "startup:1:loaded:{}",
        shell.directory.join("custom").display()
    )));
    assert!(!output.windows(5).any(|bytes| bytes == b"]133;"));
    Ok(())
}

#[test]
fn bash_is_manual_and_keeps_the_login_profile_and_prompt_command() -> TestResult {
    let mut shell = Shell::start("/bin/bash", false, true, "")?;
    assert!(
        !shell
            .until(b"muxy-test> ")?
            .windows(5)
            .any(|bytes| bytes == b"]133;")
    );
    drop(shell);
    let mut shell = Shell::start(
        "/bin/bash",
        false,
        true,
        "PROMPT_COMMAND='MUXY_TEST_PROMPT=loaded'; source \"$MUXY_SHELL_INTEGRATION_DIR/muxy.bash\"",
    )?;
    shell.until(b"\x1b]133;B\x07")?;
    shell
        .pty
        .write(b"printf 'preserved:%s\\n' \"$MUXY_TEST_PROMPT\"\n")?;
    let output = shell.until(b"\x1b]133;B\x07")?;
    assert!(String::from_utf8_lossy(&output).contains("preserved:loaded"));
    assert!(output.windows(8).any(|bytes| bytes == b"\x1b]133;C\x07"));
    shell.pty.write(b"false\n")?;
    let output = shell.until(b"\x1b]133;B\x07")?;
    assert!(
        output.windows(10).any(|bytes| bytes == b"\x1b]133;D;1\x07"),
        "{output:?}"
    );
    Ok(())
}

#[test]
fn install_rejects_symlinked_directories_and_replaces_files_without_following_links() -> TestResult
{
    let directory = std::env::temp_dir().join(format!(
        "muxy-shell-install-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory)?;
    let target = directory.join("target");
    fs::write(&target, "untouched")?;
    let hooks = directory.join("hooks");
    ShellIntegration::install(&hooks)?;
    fs::remove_file(hooks.join("muxy.zsh"))?;
    std::os::unix::fs::symlink(&target, hooks.join("muxy.zsh"))?;
    ShellIntegration::install(&hooks)?;
    assert_eq!(fs::read_to_string(&target)?, "untouched");
    let linked = directory.join("linked");
    std::os::unix::fs::symlink(&hooks, &linked)?;
    assert!(ShellIntegration::install(&linked).is_err());
    fs::remove_dir_all(&directory)?;
    Ok(())
}

#[test]
#[ignore = "requires MUXY_TEST_FISH pointing to a fish executable"]
fn fish_marks_prompts_and_preserves_user_configuration() -> TestResult {
    let program = std::env::var("MUXY_TEST_FISH")?;
    for native in [true, false] {
        let features = if native {
            "mark-prompt"
        } else {
            "no-mark-prompt"
        };
        let env = [("fish_features", features)];
        let mut shell = Shell::configured(
            &program,
            false,
            true,
            "set -g MUXY_TEST_CONFIG loaded",
            &env,
        )?;
        shell.until(b"\x1b]133;B")?;
        shell
            .pty
            .write(b"printf 'config:%s:%s\\n' $MUXY_TEST_CONFIG $__muxy_installed\n")?;
        let output = shell.until(b"\x1b]133;B")?;
        assert!(String::from_utf8_lossy(&output).contains("config:loaded:1"));
        assert!(output.windows(7).any(|bytes| bytes == b"\x1b]133;C"));
        assert!(output.windows(9).any(|bytes| bytes == b"\x1b]133;D;0"));
        shell.pty.write(b"false\n")?;
        let output = shell.until(b"\x1b]133;B")?;
        assert!(
            output.windows(9).any(|bytes| bytes == b"\x1b]133;D;1"),
            "{output:?}"
        );
        let mut shell = Shell::configured(&program, false, false, "", &env)?;
        let output = shell.until(b"muxy-test> ")?;
        if !native {
            assert!(!output.windows(5).any(|bytes| bytes == b"]133;"));
        }
        shell
            .pty
            .write(b"functions -q __muxy_directory; printf 'hooks:%s\\n' $status\n")?;
        let output = shell.until(b"hooks:1")?;
        assert!(!output.windows(8).any(|bytes| bytes == b"\x1b]133;A\x07"));
    }
    Ok(())
}

#[test]
fn bash_preserves_an_existing_debug_trap() -> TestResult {
    let mut shell = Shell::start(
        "/bin/bash",
        false,
        true,
        "trap 'MUXY_TEST_DEBUG=loaded' DEBUG; source \"$MUXY_SHELL_INTEGRATION_DIR/muxy.bash\"",
    )?;
    shell.until(b"\x1b]133;B\x07")?;
    shell
        .pty
        .write(b"printf 'debug:%s\\n' \"$MUXY_TEST_DEBUG\"\n")?;
    let output = shell.until(b"\x1b]133;B\x07")?;
    assert!(String::from_utf8_lossy(&output).contains("debug:loaded"));
    assert!(!output.windows(8).any(|bytes| bytes == b"\x1b]133;C\x07"));
    assert!(!output.windows(8).any(|bytes| bytes == b"\x1b]133;D;"));
    Ok(())
}

#[test]
fn zsh_keeps_prompt_hooks_and_percent_status_expansion() -> TestResult {
    let mut shell = Shell::start(
        "/bin/zsh",
        false,
        true,
        "precmd() { PS1='status:%? > '; }; preexec() { MUXY_TEST_PREEXEC=loaded; }",
    )?;
    shell.until(b"\x1b]133;B\x07")?;
    shell.pty.write(b"false\n")?;
    let output = shell.until(b"\x1b]133;B\x07")?;
    assert!(String::from_utf8_lossy(&output).contains("status:1 > "));
    assert!(output.windows(10).any(|bytes| bytes == b"\x1b]133;D;1\x07"));
    shell.pty.write(b"print -r -- hook:$MUXY_TEST_PREEXEC\n")?;
    assert!(String::from_utf8_lossy(&shell.until(b"\x1b]133;B\x07")?).contains("hook:loaded"));
    Ok(())
}

#[test]
fn bash_preserves_prompt_commands_with_shell_separators_and_comments() -> TestResult {
    for command in [
        "MUXY_TEST_PROMPT=loaded;",
        "MUXY_TEST_PROMPT=loaded # user prompt",
        "MUXY_TEST_PROMPT=first\nMUXY_TEST_PROMPT=loaded",
    ] {
        let mut shell = Shell::start(
            "/bin/bash",
            false,
            true,
            &format!(
                "PROMPT_COMMAND='{command}'; source \"$MUXY_SHELL_INTEGRATION_DIR/muxy.bash\""
            ),
        )?;
        shell.until(b"\x1b]133;B\x07")?;
        shell
            .pty
            .write(b"printf 'prompt:%s\\n' \"$MUXY_TEST_PROMPT\"\n")?;
        let output = shell.until(b"\x1b]133;B\x07")?;
        assert!(String::from_utf8_lossy(&output).contains("prompt:loaded"));
        shell.pty.write(b"false\n")?;
        assert!(
            shell
                .until(b"\x1b]133;B\x07")?
                .windows(10)
                .any(|bytes| bytes == b"\x1b]133;D;1\x07")
        );
    }
    Ok(())
}
