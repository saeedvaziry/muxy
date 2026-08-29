import Foundation
import Testing

@testable import Muxy

@Suite("Remote file operations")
struct RemoteFileOpsTests {
    @Test("presence probe distinguishes missing paths from inaccessible paths")
    func presenceProbeFailsClosed() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("muxy-presence-probe-\(UUID().uuidString)", isDirectory: true)
        let existing = root.appendingPathComponent("existing", isDirectory: true)
        let inaccessible = root.appendingPathComponent("inaccessible", isDirectory: true)
        try FileManager.default.createDirectory(at: existing, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: inaccessible, withIntermediateDirectories: true)
        defer {
            try? FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: inaccessible.path)
            try? FileManager.default.removeItem(at: root)
        }

        let existingResult = try await runPresenceProbe(existing.path)
        let missingResult = try await runPresenceProbe(root.appendingPathComponent("missing/nested").path)

        try FileManager.default.setAttributes([.posixPermissions: 0o000], ofItemAtPath: inaccessible.path)
        let inaccessibleResult = try await runPresenceProbe(inaccessible.appendingPathComponent("nested").path)

        #expect(existingResult.status == 0)
        #expect(missingResult.status == 1)
        #expect(inaccessibleResult.status == 2)
    }

    private func runPresenceProbe(_ path: String) async throws -> SubprocessResult {
        try await SubprocessRunner.run(SubprocessRequest(
            executablePath: "/bin/sh",
            arguments: ["-c", SSHFileOps.presenceCommand(path: path)]
        ))
    }
}
