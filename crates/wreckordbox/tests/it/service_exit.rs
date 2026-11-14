//! A command that arrives while the Library's last service is still going
//! (wbx-1.2): it has stopped answering but still holds the Library's lock,
//! as a crashing service does while its core is written. The service is
//! held in that state by the test-only `WRECKORDBOX_TEST_HOLD_LOCK` fault,
//! and let go when the test says.

use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use wreckordbox_service::release_wait::{HOLD_LOCK_ENV, RELEASE_WAIT_ENV};
use wreckordbox_service::{codes, library_released, Client};

use crate::common::{Run, Sandbox};

/// A service started with the hold fault, stopped, and now held: closed,
/// still owning the Library, with the socket file a crash leaves behind
/// (present, refusing). Answers the old service's pid and the gate file
/// that lets it go.
fn a_service_held_while_exiting(sandbox: &Sandbox) -> (u64, PathBuf) {
    let gate = sandbox.path().join("let-go");
    let first = sandbox.run(
        sandbox
            .command()
            .env(HOLD_LOCK_ENV, &gate)
            .args(["--json", "status"]),
    );
    assert!(first.status.success(), "{}", first.stderr);
    let old = first.json()["pid"].as_u64().unwrap();

    sandbox.client().stop().unwrap();
    let held = PathBuf::from(format!("{}.held", gate.display()));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !held.exists() {
        assert!(
            Instant::now() < deadline,
            "the service never reached the hold"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !library_released(sandbox.master_db()),
        "the hold must keep the lock"
    );

    // What a crashed service leaves: its socket file, which nobody answers.
    let socket = sandbox.config().socket_path(sandbox.location());
    drop(UnixListener::bind(&socket).unwrap());
    assert!(socket.exists());
    assert_eq!(
        Client::connect(&socket).unwrap_err().code,
        codes::SERVICE_NOT_RUNNING
    );
    (old, gate)
}

fn spawn_status(sandbox: &Sandbox, release_wait_secs: Option<u64>) -> Child {
    let mut command = sandbox.command();
    command
        .args(["--json", "status"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(secs) = release_wait_secs {
        command.env(RELEASE_WAIT_ENV, secs.to_string());
    }
    command.spawn().unwrap()
}

fn finished(child: Child) -> Run {
    let output = child.wait_with_output().unwrap();
    Run {
        status: output.status,
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

fn service_log(sandbox: &Sandbox) -> String {
    std::fs::read_to_string(sandbox.runtime_dir().join("service.log")).unwrap_or_default()
}

/// Waits until the service the new command started says it is waiting
/// for the old one, or the command has already finished (as it does,
/// failing, without the fix).
fn until_waiting_or_done(sandbox: &Sandbox, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !service_log(sandbox).contains("waiting up to")
        && child.try_wait().unwrap().is_none()
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn let_go(gate: &Path) {
    std::fs::write(gate, b"").unwrap();
}

#[test]
fn a_command_while_a_dying_service_still_holds_the_library_succeeds_once_it_lets_go() {
    let sandbox = Sandbox::new();
    let (old, gate) = a_service_held_while_exiting(&sandbox);

    let mut command = spawn_status(&sandbox, None);
    until_waiting_or_done(&sandbox, &mut command);
    let_go(&gate);
    let started = Instant::now();
    let run = finished(command);

    assert!(run.status.success(), "{}", run.stderr);
    let new = run.json()["pid"].as_u64().unwrap();
    assert_ne!(new, old, "a new service has the Library");
    // It arrived while the old one held the Library, and said it waited.
    assert!(
        service_log(&sandbox).contains(&format!("(pid {old}) no longer answers")),
        "{}",
        service_log(&sandbox)
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "started once the Library was let go, not after a timeout: {:?}",
        started.elapsed()
    );
    // The stale socket was replaced: a second client reaches the new one.
    assert_eq!(
        sandbox.client().status().unwrap().pid,
        u32::try_from(new).unwrap()
    );
    assert!(
        !library_released(sandbox.master_db()),
        "the new service owns the Library"
    );
}

#[test]
fn a_service_that_never_lets_go_is_named_in_the_failure() {
    let sandbox = Sandbox::new();
    let (old, gate) = a_service_held_while_exiting(&sandbox);

    let started = Instant::now();
    let run = finished(spawn_status(&sandbox, Some(1)));
    let waited = started.elapsed();
    let_go(&gate);

    assert_eq!(run.status.code(), Some(1), "{}", run.stdout);
    let error = run.error_json();
    assert_eq!(
        error["error"]["code"],
        codes::SERVICE_ALREADY_RUNNING,
        "{error}"
    );
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains(&format!("pid {old}")), "{message}");
    assert!(message.contains("did not let go"), "{message}");
    assert!(
        waited >= Duration::from_secs(1),
        "the configured wait was waited: {waited:?}"
    );
    assert!(
        waited < Duration::from_secs(8),
        "bounded by the wait: {waited:?}"
    );
}

#[test]
fn a_live_service_is_still_refused_at_once_and_keeps_the_library() {
    let sandbox = Sandbox::new();
    let running = sandbox.status();

    // A client connects to it at once.
    let started = Instant::now();
    assert_eq!(sandbox.status()["pid"], running["pid"]);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "a client connects at once: {:?}",
        started.elapsed()
    );

    let started = Instant::now();
    let second = sandbox.run(sandbox.command().env(RELEASE_WAIT_ENV, "30").arg("serve"));

    assert_eq!(second.status.code(), Some(1));
    assert!(
        second.stderr.contains("already running"),
        "{}",
        second.stderr
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a live service answers, so nothing is waited for: {:?}",
        started.elapsed()
    );
    assert_eq!(sandbox.status()["pid"], running["pid"]);
}

/// A holder that keeps its lock and its listening socket but never accepts
/// (paused, or stuck), with its accept backlog full: a second `serve` must
/// not wait on it past its bound. It is a live listener, so the answer is
/// the refusal it always was, at once.
#[test]
fn a_holder_whose_socket_never_accepts_does_not_hold_up_a_second_serve() {
    use socket2::{Domain, SockAddr, Socket, Type};

    let sandbox = Sandbox::new();
    let socket = sandbox.config().socket_path(sandbox.location());
    std::fs::create_dir_all(sandbox.runtime_dir()).unwrap();

    // The holder: the Library's lock, with the note a service writes.
    let lock = std::fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(
            sandbox
                .master_db()
                .with_file_name(".wreckordbox-service.lock"),
        )
        .unwrap();
    lock.try_lock().unwrap();
    std::fs::write(
        sandbox
            .master_db()
            .with_file_name(".wreckordbox-service.lock"),
        format!("{}\n{}\n", std::process::id(), socket.display()),
    )
    .unwrap();
    let listener = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap();
    listener.bind(&SockAddr::unix(&socket).unwrap()).unwrap();
    listener.listen(0).unwrap();
    // Fill the backlog: connect without blocking until it is refused.
    let mut pending = Vec::new();
    loop {
        let client = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap();
        client.set_nonblocking(true).unwrap();
        match client.connect(&SockAddr::unix(&socket).unwrap()) {
            Ok(()) => pending.push(client),
            Err(_) => break,
        }
        assert!(pending.len() < 10_000, "the backlog never filled");
    }

    let started = Instant::now();
    let mut serve = sandbox
        .command()
        .env(RELEASE_WAIT_ENV, "2")
        .arg("serve")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Its own watchdog: a hang fails here instead of hanging the suite.
    let watchdog = Duration::from_secs(15);
    while serve.try_wait().unwrap().is_none() {
        if started.elapsed() > watchdog {
            let _ = serve.kill();
            let _ = serve.wait();
            panic!("serve was still waiting after {watchdog:?}, past its 2 s bound");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let waited = started.elapsed();
    let run = finished(serve);

    assert_eq!(run.status.code(), Some(1), "{}", run.stdout);
    assert!(run.stderr.contains("already running"), "{}", run.stderr);
    assert!(
        run.stderr.contains(&format!("pid {}", std::process::id())),
        "{}",
        run.stderr
    );
    assert!(
        waited < Duration::from_secs(2),
        "a live listener is refused at once: {waited:?}"
    );
    // Nothing was taken from the holder or removed.
    assert!(!library_released(sandbox.master_db()));
    assert!(socket.exists());
    drop((pending, listener, lock));
}
