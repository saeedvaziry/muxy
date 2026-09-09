import Foundation
import Testing

@testable import Muxy

@Suite("WorkspacePathResolver")
struct WorkspacePathResolverTests {
    @Test("local paths resolve relative to the repository and through symlinks")
    func resolvesLocalPaths() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("muxy-path-resolver-\(UUID().uuidString)", isDirectory: true)
        let repository = root.appendingPathComponent("repos/app", isDirectory: true)
        let worktree = root.appendingPathComponent("repos/app-feature", isDirectory: true)
        let alias = root.appendingPathComponent("feature-alias", isDirectory: true)
        try FileManager.default.createDirectory(at: repository, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: worktree, withIntermediateDirectories: true)
        try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: worktree)
        defer { try? FileManager.default.removeItem(at: root) }

        let resolutions = try await WorkspacePathResolver.live.resolve(
            paths: ["../app-feature", alias.path],
            relativeTo: repository.path,
            context: .local,
            timeout: 1
        )
        let physicalWorktreePath = worktree.resolvingSymlinksInPath().path

        #expect(resolutions.map(\.path) == [physicalWorktreePath, physicalWorktreePath])
    }

    @Test("SSH paths resolve home, repository-relative paths, symlinks, and missing descendants")
    func resolvesRemotePaths() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("muxy-remote-path-resolver-\(UUID().uuidString)", isDirectory: true)
        let home = root.appendingPathComponent("home", isDirectory: true)
        let repository = home.appendingPathComponent("repos/app", isDirectory: true)
        let worktree = home.appendingPathComponent("repos/app feature's", isDirectory: true)
        let alias = home.appendingPathComponent("worktree-alias", isDirectory: true)
        try FileManager.default.createDirectory(at: repository, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: worktree, withIntermediateDirectories: true)
        try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: worktree)
        defer { try? FileManager.default.removeItem(at: root) }

        let resolver = WorkspacePathResolver { _, command, timeout in
            var environment = ProcessInfo.processInfo.environment
            environment["HOME"] = home.path
            let result = try await SubprocessRunner.run(SubprocessRequest(
                executablePath: "/bin/sh",
                arguments: ["-c", command],
                workingDirectory: home.path,
                environment: environment,
                timeout: timeout
            ))
            return WorkspacePathCommandResult(
                status: result.status,
                stdoutData: result.stdoutData,
                stderr: result.stderr
            )
        }
        let destination = SSHDestination(host: "example.com")
        let pwdResult = try await SubprocessRunner.run(SubprocessRequest(
            executablePath: "/bin/pwd",
            workingDirectory: home.path
        ))
        let physicalHome = pwdResult.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
        let resolvedWorktree = physicalHome + "/repos/app feature's"
        let missing = physicalHome + "/repos/missing/nested"

        let resolutions = try await resolver.resolve(
            paths: [
                "~/repos/app feature's",
                "../app feature's",
                alias.path,
                "~/repos/missing/nested",
                "~/Code/org/.muxy-worktrees/app/feature"
            ],
            relativeTo: "~/repos/app",
            context: .ssh(destination),
            timeout: 5
        )

        #expect(resolutions.map(\.path) == [
            resolvedWorktree,
            resolvedWorktree,
            resolvedWorktree,
            missing,
            physicalHome + "/Code/org/.muxy-worktrees/app/feature"
        ])
    }

    @Test("absolute SSH paths do not require HOME")
    func resolvesAbsoluteRemotePathWithoutHome() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("muxy-absolute-remote-path-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }

        let resolver = WorkspacePathResolver { _, command, timeout in
            var environment = ProcessInfo.processInfo.environment
            environment["HOME"] = ""
            let result = try await SubprocessRunner.run(SubprocessRequest(
                executablePath: "/bin/sh",
                arguments: ["-c", command],
                workingDirectory: directory.path,
                environment: environment,
                timeout: timeout
            ))
            return WorkspacePathCommandResult(
                status: result.status,
                stdoutData: result.stdoutData,
                stderr: result.stderr
            )
        }

        let resolutions = try await resolver.resolve(
            paths: [directory.path],
            relativeTo: "~/repo",
            context: .ssh(SSHDestination(host: "example.com")),
            timeout: 5
        )
        let pwdResult = try await SubprocessRunner.run(SubprocessRequest(
            executablePath: "/bin/pwd",
            workingDirectory: directory.path
        ))

        #expect(resolutions.map(\.path) == [pwdResult.stdout.trimmingCharacters(in: .whitespacesAndNewlines)])
    }

    @Test("large SSH path sets are resolved in bounded batches")
    func batchesLargeRemotePathSets() async throws {
        let paths = (0 ..< 800).map { index in
            "/tmp/muxy-batch-\(index)-\(String(repeating: "a", count: 80))"
        }
        let counter = WorkspacePathRunnerCounter()
        let resolver = WorkspacePathResolver { _, command, _ in
            await counter.record(command)
            let batchPaths = command.split(separator: " ").filter { $0.hasPrefix("/tmp/muxy-batch-") }
            return WorkspacePathCommandResult(
                status: 0,
                stdoutData: Data(batchPaths.map { "\($0)\0" }.joined().utf8),
                stderr: ""
            )
        }

        let resolutions = try await resolver.resolve(
            paths: paths,
            relativeTo: "/tmp/repo",
            context: .ssh(SSHDestination(host: "example.com")),
            timeout: 10
        )

        #expect(resolutions.map(\.path) == paths)
        #expect(await counter.value > 1)
        #expect(await counter.maximumCommandBytes <= 32 * 1024)
    }

    @Test("SSH command failures use resolver errors")
    func reportsResolverError() async {
        let resolver = WorkspacePathResolver { _, _, _ in
            WorkspacePathCommandResult(status: 1, stdoutData: Data(), stderr: "permission denied")
        }

        await #expect(throws: WorkspacePathResolverError.commandFailed("permission denied")) {
            try await resolver.resolve(
                paths: ["~/repo"],
                relativeTo: "~",
                context: .ssh(SSHDestination(host: "example.com")),
                timeout: 1
            )
        }
    }
}

private actor WorkspacePathRunnerCounter {
    private(set) var value = 0
    private(set) var maximumCommandBytes = 0

    func record(_ command: String) {
        value += 1
        maximumCommandBytes = max(maximumCommandBytes, command.utf8.count)
    }
}
