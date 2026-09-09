import Darwin
import Foundation
import MuxySessionProtocol
import Testing

@testable import Muxy

@Suite("SessionDaemon end to end", .serialized, .enabled(if: SessionDaemonHarness.binaryURL != nil))
struct SessionDaemonEndToEndTests {
    private struct SignalResistantSession {
        let identifier: SessionIdentifier
        let connection: SessionTestConnection
        let processIDs: [pid_t]
    }

    private func makeIdentifier() throws -> SessionIdentifier {
        try #require(SessionIdentifier(uuidString: UUID().uuidString))
    }

    private func withHarness(_ body: (SessionDaemonHarness) throws -> Void) throws {
        let harness = try SessionDaemonHarness()
        try harness.start()
        defer { harness.stop() }
        try body(harness)
    }

    @Test("creates a session and streams its output")
    func createsSessionAndStreamsOutput() throws {
        try withHarness { harness in
            let identifier = try makeIdentifier()
            let connection = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { connection.close() }

            connection.send(harness.attachRequest(identifier: identifier, command: "sh -c 'echo READY; sleep 30'"))

            let attached = try #require(connection.waitForFrame(timeout: 5) { $0.kind == .attached })
            let acceptance = try SessionAttachAccepted.decode(attached.payload)
            #expect(acceptance.created)
            #expect(acceptance.shellProcessID > 0)
            #expect(acceptance.ttyDevice != 0)

            let output = connection.collectOutput(timeout: 5) { $0.contains("READY") }
            #expect(output.contains("READY"))
        }
    }

    @Test("keeps the session alive after the client disconnects and replays on reattach")
    func replaysAfterReattach() throws {
        try withHarness { harness in
            let identifier = try makeIdentifier()
            let first = try #require(SessionTestConnection(socketPath: harness.socketPath))
            first.send(harness.attachRequest(identifier: identifier, command: "sh -c 'echo MARKER; sleep 30'"))
            _ = first.collectOutput(timeout: 5) { $0.contains("MARKER") }
            first.close()

            let second = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { second.close() }
            second.send(harness.attachRequest(identifier: identifier, command: "sh -c 'echo SHOULD_NOT_RUN; sleep 30'"))

            let attached = try #require(second.waitForFrame(timeout: 5) { $0.kind == .attached })
            let acceptance = try SessionAttachAccepted.decode(attached.payload)
            #expect(!acceptance.created)

            let replayed = second.collectOutput(timeout: 5) { $0.contains("MARKER") }
            #expect(replayed.contains("MARKER"))
            #expect(!replayed.contains("SHOULD_NOT_RUN"))
        }
    }

    @Test("attach client does not receive response-generating Kitty commands from replay")
    func attachClientFiltersKittyCommandsDuringReplay() throws {
        try withHarness { harness in
            let identifier = try makeIdentifier()
            let query = "\u{1B}_Ga=t,i=1,s=1,v=1,f=24;AAAA\u{1B}\\"
            let first = try #require(SessionTestConnection(socketPath: harness.socketPath))
            first.send(harness.attachRequest(
                identifier: identifier,
                command: "sh -c \"printf 'BEFORE\\033_Ga=t,i=1,s=1,v=1,f=24;AAAA\\033\\\\AFTER'; sleep 30\""
            ))
            let initialOutput = first.collectOutput(timeout: 5) { $0.contains("AFTER") }
            #expect(initialOutput.contains(query))
            first.close()

            let replayed = try harness.runAttachClient(
                identifier: identifier,
                command: "echo SHOULD_NOT_RUN",
                timeout: 1
            ).output
            #expect(replayed.contains("BEFOREAFTER"))
            #expect(!replayed.contains(query))
            #expect(!replayed.contains("SHOULD_NOT_RUN"))
        }
    }

    @Test("uses fallback pty size for a new session with a transient attach size")
    func usesFallbackSizeForTransientInitialAttach() throws {
        try withHarness { harness in
            let identifier = try makeIdentifier()
            let connection = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { connection.close() }

            connection.send(harness.attachRequest(
                identifier: identifier,
                command: "",
                columns: 1,
                rows: 1
            ))
            _ = try #require(connection.waitForFrame(timeout: 5) { $0.kind == .attached })

            connection.send(SessionFrame(kind: .input, payload: Array("stty size\n".utf8)))
            let output = connection.collectOutput(timeout: 5) { $0.contains("24 80") || $0.contains("1 1") }
            #expect(output.contains("24 80"))
        }
    }

    @Test("does not resize an existing session from a transient reattach size")
    func ignoresTransientSizeForExistingSessionReattach() throws {
        try withHarness { harness in
            let identifier = try makeIdentifier()
            let first = try #require(SessionTestConnection(socketPath: harness.socketPath))
            first.send(harness.attachRequest(identifier: identifier, command: "", columns: 80, rows: 24))
            _ = try #require(first.waitForFrame(timeout: 5) { $0.kind == .attached })
            first.send(SessionFrame(kind: .input, payload: Array("stty size\n".utf8)))
            _ = first.collectOutput(timeout: 5) { $0.contains("24 80") || $0.contains("1 1") }
            first.close()

            let second = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { second.close() }
            second.send(harness.attachRequest(identifier: identifier, command: "", columns: 1, rows: 1))
            _ = try #require(second.waitForFrame(timeout: 5) { $0.kind == .attached })
            second.send(SessionFrame(kind: .input, payload: Array("stty size\n".utf8)))
            let output = second.collectOutput(timeout: 5) { $0.contains("24 80") || $0.contains("1 1") }
            #expect(output.contains("24 80"))
            #expect(!output.contains("1 1"))
        }
    }

    @Test("forwards input to the session")
    func forwardsInput() throws {
        try withHarness { harness in
            let identifier = try makeIdentifier()
            let connection = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { connection.close() }

            connection.send(harness.attachRequest(
                identifier: identifier,
                command: "sh -c 'while read line; do echo GOT:$line; done'"
            ))
            _ = try #require(connection.waitForFrame(timeout: 5) { $0.kind == .attached })

            connection.send(SessionFrame(kind: .input, payload: Array("ping\n".utf8)))
            let output = connection.collectOutput(timeout: 5) { $0.contains("GOT:ping") }
            #expect(output.contains("GOT:ping"))
        }
    }

    @Test("runs a pane startup command through the shell wrapper the app builds")
    func runsStartupCommandWrapper() throws {
        try withHarness { harness in
            let identifier = try makeIdentifier()
            let connection = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { connection.close() }

            let wrapper = TerminalLaunchCommand.shellCommand(
                interactive: false,
                keepsShellOpen: true,
                shell: "/bin/sh"
            )
            connection.send(harness.attachRequest(
                identifier: identifier,
                command: wrapper,
                environment: [
                    SessionEnvironmentEntry(
                        key: TerminalLaunchCommand.environmentKey,
                        value: "echo STARTUP_RAN"
                    ),
                ]
            ))

            let output = connection.collectOutput(timeout: 5) { $0.contains("STARTUP_RAN") }
            #expect(output.contains("STARTUP_RAN"))
        }
    }

    @Test("keeps the shell open after a startup command that is meant to persist")
    func keepsShellOpenAfterStartupCommand() throws {
        try withHarness { harness in
            let identifier = try makeIdentifier()
            let connection = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { connection.close() }

            connection.send(harness.attachRequest(
                identifier: identifier,
                command: TerminalLaunchCommand.shellCommand(
                    interactive: false,
                    keepsShellOpen: true,
                    shell: "/bin/sh"
                ),
                environment: [
                    SessionEnvironmentEntry(key: TerminalLaunchCommand.environmentKey, value: "true"),
                ]
            ))
            _ = try #require(connection.waitForFrame(timeout: 5) { $0.kind == .attached })

            let control = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { control.close() }
            control.send(SessionFrame(kind: .list))
            let listed = try #require(control.waitForFrame(timeout: 5) { $0.kind == .sessions })
            #expect(try SessionDescriptor.decodeList(listed.payload).count == 1)
        }
    }

    @Test("reports the exit status when the session ends")
    func reportsExitStatus() throws {
        try withHarness { harness in
            let identifier = try makeIdentifier()
            let connection = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { connection.close() }

            connection.send(harness.attachRequest(identifier: identifier, command: "sh -c 'exit 7'"))
            let exited = try #require(connection.waitForFrame(timeout: 5) { $0.kind == .exited })
            #expect(try SessionExitPayload.decode(exited.payload) == 7)
        }
    }

    @Test("lists live sessions with their tty and shell process")
    func listsSessions() throws {
        try withHarness { harness in
            let identifier = try makeIdentifier()
            let attached = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { attached.close() }
            attached.send(harness.attachRequest(identifier: identifier, command: "sh -c 'sleep 30'"))
            _ = try #require(attached.waitForFrame(timeout: 5) { $0.kind == .attached })

            let control = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { control.close() }
            control.send(SessionFrame(kind: .list))
            let listed = try #require(control.waitForFrame(timeout: 5) { $0.kind == .sessions })
            let descriptors = try SessionDescriptor.decodeList(listed.payload)

            #expect(descriptors.count == 1)
            let descriptor = try #require(descriptors.first)
            #expect(descriptor.identifier == identifier)
            #expect(descriptor.shellProcessID > 0)
            #expect(descriptor.ttyDevice != 0)
            #expect(descriptor.workingDirectory == "/tmp")
            #expect(descriptor.isAttached)
        }
    }

    @Test("reports a session as detached once its client goes away")
    func reportsDetachedSessions() throws {
        try withHarness { harness in
            let identifier = try makeIdentifier()
            let attached = try #require(SessionTestConnection(socketPath: harness.socketPath))
            attached.send(harness.attachRequest(identifier: identifier, command: "sh -c 'sleep 30'"))
            _ = try #require(attached.waitForFrame(timeout: 5) { $0.kind == .attached })
            attached.close()

            let control = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { control.close() }

            var descriptors: [SessionDescriptor] = []
            let deadline = Date().addingTimeInterval(5)
            while Date() < deadline {
                control.send(SessionFrame(kind: .list))
                let listed = try #require(control.waitForFrame(timeout: 5) { $0.kind == .sessions })
                descriptors = try SessionDescriptor.decodeList(listed.payload)
                if descriptors.first?.isAttached == false { break }
                usleep(50_000)
            }
            #expect(descriptors.count == 1)
            #expect(descriptors.first?.isAttached == false)
        }
    }

    @Test("kills a session on request")
    func killsSession() throws {
        try withHarness { harness in
            let identifier = try makeIdentifier()
            let attached = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { attached.close() }
            attached.send(harness.attachRequest(identifier: identifier, command: "sh -c 'sleep 30'"))
            _ = try #require(attached.waitForFrame(timeout: 5) { $0.kind == .attached })

            let control = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { control.close() }
            control.send(SessionFrame(kind: .kill, payload: SessionIdentifierPayload.encode(identifier)))
            _ = try #require(control.waitForFrame(timeout: 5) { $0.kind == .acknowledged })

            _ = try #require(attached.waitForFrame(timeout: 5) { $0.kind == .exited })

            control.send(SessionFrame(kind: .list))
            let listed = try #require(control.waitForFrame(timeout: 5) { $0.kind == .sessions })
            #expect(try SessionDescriptor.decodeList(listed.payload).isEmpty)
        }
    }

    @Test("kills an entire session whose processes ignore hangup and termination")
    func killsSignalResistantSession() throws {
        try withHarness { harness in
            let resistantSession = try makeSignalResistantSession(harness: harness)
            defer { resistantSession.connection.close() }

            let control = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { control.close() }
            control.send(SessionFrame(
                kind: .kill,
                payload: SessionIdentifierPayload.encode(resistantSession.identifier)
            ))
            _ = try #require(control.waitForFrame(timeout: 5) { $0.kind == .acknowledged })

            for processID in resistantSession.processIDs {
                #expect(!processExists(processID))
            }
        }
    }

    @Test("kills signal-resistant sessions together within the control timeout")
    func killsAllSignalResistantSessionsWithinControlTimeout() throws {
        try withHarness { harness in
            var resistantSessions: [SignalResistantSession] = []
            defer { resistantSessions.forEach { $0.connection.close() } }
            for _ in 0 ..< 12 {
                resistantSessions.append(try makeSignalResistantSession(harness: harness))
            }

            let control = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { control.close() }
            control.send(SessionFrame(kind: .killAll))
            _ = try #require(control.waitForFrame(timeout: PersistentSessionControlClient.defaultTimeout) {
                $0.kind == .acknowledged
            })

            for processID in resistantSessions.flatMap(\.processIDs) {
                #expect(!processExists(processID))
            }
        }
    }

    @Test("returns session details for a single identifier")
    func returnsSessionInfo() throws {
        try withHarness { harness in
            let identifier = try makeIdentifier()
            let other = try makeIdentifier()
            let attached = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { attached.close() }
            attached.send(harness.attachRequest(identifier: identifier, command: "sh -c 'sleep 30'"))
            _ = try #require(attached.waitForFrame(timeout: 5) { $0.kind == .attached })

            let control = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { control.close() }

            control.send(SessionFrame(kind: .info, payload: SessionIdentifierPayload.encode(identifier)))
            let found = try #require(control.waitForFrame(timeout: 5) { $0.kind == .sessions })
            #expect(try SessionDescriptor.decodeList(found.payload).count == 1)

            control.send(SessionFrame(kind: .info, payload: SessionIdentifierPayload.encode(other)))
            let missing = try #require(control.waitForFrame(timeout: 5) { $0.kind == .sessions })
            #expect(try SessionDescriptor.decodeList(missing.payload).isEmpty)
        }
    }

    @Test("reports which tab, project and worktree a session belongs to")
    func reportsSessionOwnership() throws {
        try withHarness { harness in
            let identifier = try makeIdentifier()
            let projectID = UUID().uuidString
            let worktreeID = UUID().uuidString
            let tabID = UUID().uuidString

            let attached = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { attached.close() }
            attached.send(harness.attachRequest(
                identifier: identifier,
                command: "sh -c 'sleep 30'",
                metadata: [
                    SessionEnvironmentEntry(key: SessionMetadataKey.project, value: projectID),
                    SessionEnvironmentEntry(key: SessionMetadataKey.worktree, value: worktreeID),
                    SessionEnvironmentEntry(key: SessionMetadataKey.tab, value: tabID),
                    SessionEnvironmentEntry(key: SessionMetadataKey.title, value: "Dev Server"),
                ]
            ))
            _ = try #require(attached.waitForFrame(timeout: 5) { $0.kind == .attached })

            let control = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { control.close() }
            control.send(SessionFrame(kind: .list))
            let listed = try #require(control.waitForFrame(timeout: 5) { $0.kind == .sessions })
            let descriptor = try #require(try SessionDescriptor.decodeList(listed.payload).first)

            #expect(descriptor.value(forMetadataKey: SessionMetadataKey.project) == projectID)
            #expect(descriptor.value(forMetadataKey: SessionMetadataKey.worktree) == worktreeID)
            #expect(descriptor.value(forMetadataKey: SessionMetadataKey.tab) == tabID)
            #expect(descriptor.value(forMetadataKey: SessionMetadataKey.title) == "Dev Server")
        }
    }

    @Test("refreshes ownership when a session is reattached from another tab")
    func refreshesOwnershipOnReattach() throws {
        try withHarness { harness in
            let identifier = try makeIdentifier()
            let first = try #require(SessionTestConnection(socketPath: harness.socketPath))
            first.send(harness.attachRequest(
                identifier: identifier,
                command: "sh -c 'sleep 30'",
                metadata: [SessionEnvironmentEntry(key: SessionMetadataKey.tab, value: "first-tab")]
            ))
            _ = try #require(first.waitForFrame(timeout: 5) { $0.kind == .attached })
            first.close()

            let second = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { second.close() }
            second.send(harness.attachRequest(
                identifier: identifier,
                command: "",
                metadata: [SessionEnvironmentEntry(key: SessionMetadataKey.tab, value: "second-tab")]
            ))
            _ = try #require(second.waitForFrame(timeout: 5) { $0.kind == .attached })

            let control = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { control.close() }
            control.send(SessionFrame(kind: .info, payload: SessionIdentifierPayload.encode(identifier)))
            let found = try #require(control.waitForFrame(timeout: 5) { $0.kind == .sessions })
            let descriptor = try #require(try SessionDescriptor.decodeList(found.payload).first)
            #expect(descriptor.value(forMetadataKey: SessionMetadataKey.tab) == "second-tab")
        }
    }

    @Test("refuses an attach from a different protocol version")
    func refusesVersionMismatch() throws {
        try withHarness { harness in
            let identifier = try makeIdentifier()
            let connection = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { connection.close() }
            connection.send(harness.attachRequest(
                identifier: identifier,
                command: "sh -c 'sleep 30'",
                version: SessionProtocolVersion.current &+ 1
            ))
            let failure = try #require(connection.waitForFrame(timeout: 5) { $0.kind == .failure })
            #expect(try SessionTextPayload.decode(failure.payload).contains("different version"))

            let control = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { control.close() }
            control.send(SessionFrame(kind: .list))
            let listed = try #require(control.waitForFrame(timeout: 5) { $0.kind == .sessions })
            #expect(try SessionDescriptor.decodeList(listed.payload).isEmpty)
        }
    }

    @Test("reports a failure for a malformed attach request")
    func reportsMalformedAttach() throws {
        try withHarness { harness in
            let connection = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { connection.close() }
            connection.send(SessionFrame(kind: .attach, payload: [1, 2, 3]))
            let failure = try #require(connection.waitForFrame(timeout: 5) { $0.kind == .failure })
            #expect(try SessionTextPayload.decode(failure.payload).isEmpty == false)
        }
    }

    @Test("hands a reattaching client the session and drops the previous one")
    func replacesPreviousClient() throws {
        try withHarness { harness in
            let identifier = try makeIdentifier()
            let first = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { first.close() }
            first.send(harness.attachRequest(identifier: identifier, command: "sh -c 'sleep 30'"))
            _ = try #require(first.waitForFrame(timeout: 5) { $0.kind == .attached })

            let second = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { second.close() }
            second.send(harness.attachRequest(identifier: identifier, command: "sh -c 'sleep 30'"))
            let attached = try #require(second.waitForFrame(timeout: 5) { $0.kind == .attached })
            #expect(try SessionAttachAccepted.decode(attached.payload).created == false)

            let control = try #require(SessionTestConnection(socketPath: harness.socketPath))
            defer { control.close() }
            control.send(SessionFrame(kind: .list))
            let listed = try #require(control.waitForFrame(timeout: 5) { $0.kind == .sessions })
            #expect(try SessionDescriptor.decodeList(listed.payload).count == 1)
        }
    }

    private func processExists(_ processID: pid_t) -> Bool {
        guard kill(processID, 0) != 0 else { return true }
        return errno == EPERM
    }

    private func makeSignalResistantSession(harness: SessionDaemonHarness) throws -> SignalResistantSession {
        let identifier = try makeIdentifier()
        let connection = try #require(SessionTestConnection(socketPath: harness.socketPath))
        connection.send(harness.attachRequest(
            identifier: identifier,
            command: "sh -c 'trap \"\" HUP TERM; sh -c \"trap \\\"\\\" HUP TERM; echo CHILD=\\$\\$; while :; do sleep 1; done\" & echo READY; wait'"
        ))

        let acceptedFrame = try #require(connection.waitForFrame(timeout: 5) { $0.kind == .attached })
        let accepted = try SessionAttachAccepted.decode(acceptedFrame.payload)
        let output = connection.collectOutput(timeout: 5) { $0.contains("READY") && $0.contains("CHILD=") }
        let childProcessID = output
            .split(whereSeparator: \.isNewline)
            .lazy
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .first { $0.hasPrefix("CHILD=") }
            .flatMap { pid_t(String($0.dropFirst("CHILD=".count))) }

        return try SignalResistantSession(
            identifier: identifier,
            connection: connection,
            processIDs: [accepted.shellProcessID, #require(childProcessID)]
        )
    }
}
