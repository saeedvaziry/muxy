import Foundation
import Testing

@testable import Muxy

@Suite("GitWorktreeService.removeWorktree")
struct GitWorktreeServiceRemoveTests {
    @Test("removes a normal worktree")
    func removesNormalWorktree() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }

        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("feature-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "feature",
            createBranch: true,
            baseBranch: nil
        )

        try await GitWorktreeService.shared.removeWorktree(repoPath: repo.path, path: worktreePath, force: true)

        let records = try await GitWorktreeService.shared.listWorktrees(repoPath: repo.path)
        #expect(!records.contains { $0.path == worktreePath })
    }

    @Test("resolves repository-relative paths before removal")
    func removesRepositoryRelativeWorktree() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }

        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("relative-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "relative-feature",
            createBranch: true,
            baseBranch: nil
        )

        let removedPath = try await GitWorktreeService.shared.removeWorktree(
            repoPath: repo.path,
            path: "../relative-wt",
            force: true
        )

        #expect(removedPath == URL(fileURLWithPath: worktreePath).resolvingSymlinksInPath().path)
        let records = try await GitWorktreeService.shared.listWorktrees(repoPath: repo.path)
        #expect(!records.contains { $0.path == worktreePath })
    }

    @Test("removes a worktree when the repository path is another linked worktree")
    func removesFromLinkedWorktreeRepositoryPath() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }
        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let linkedRepoPath = repo.siblingPath("linked-repository-wt")
        let worktreePath = repo.siblingPath("linked-repository-target-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: linkedRepoPath,
            branch: "linked-repository-feature",
            createBranch: true,
            baseBranch: nil
        )
        try await GitWorktreeService.shared.addWorktree(
            repoPath: linkedRepoPath,
            path: worktreePath,
            branch: "linked-repository-target-feature",
            createBranch: true,
            baseBranch: nil
        )

        try await GitWorktreeService.shared.removeWorktree(
            repoPath: linkedRepoPath,
            path: worktreePath,
            force: true
        )

        #expect(!FileManager.default.fileExists(atPath: worktreePath))
    }

    @Test("rejects the primary worktree without stopping its processes")
    func rejectsPrimaryWorktreeWithoutStoppingProcesses() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/sleep")
        process.arguments = ["30"]
        process.currentDirectoryURL = URL(fileURLWithPath: repo.path)
        try process.run()
        defer {
            if process.isRunning {
                process.terminate()
            }
        }

        await #expect(throws: Error.self) {
            try await GitWorktreeService.shared.removeWorktree(
                repoPath: repo.path,
                path: repo.path,
                force: true
            )
        }

        #expect(process.isRunning)
        #expect(FileManager.default.fileExists(atPath: repo.path))
    }

    @Test("stops a process writing inside the worktree before removal")
    func stopsActiveWriterBeforeRemoval() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }

        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("active-writer-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "active-writer-feature",
            createBranch: true,
            baseBranch: nil
        )
        let writer = Process()
        writer.executableURL = URL(fileURLWithPath: "/bin/sh")
        writer.arguments = ["-c", "while :; do mkdir -p generated; touch generated/$RANDOM; done"]
        writer.currentDirectoryURL = URL(fileURLWithPath: worktreePath)
        try writer.run()
        defer {
            if writer.isRunning {
                writer.terminate()
            }
        }

        try await GitWorktreeService.shared.removeWorktree(
            repoPath: repo.path,
            path: worktreePath,
            force: true
        )

        #expect(!writer.isRunning)
        #expect(!FileManager.default.fileExists(atPath: worktreePath))
        let records = try await GitWorktreeService.shared.listWorktrees(repoPath: repo.path)
        #expect(!records.contains { $0.path == worktreePath })
    }

    @Test("removes a residual directory after Git partially deregisters a worktree")
    func removesResidualDirectoryAfterPartialRemoval() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }

        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("partial-removal-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "partial-removal-feature",
            createBranch: true,
            baseBranch: nil
        )

        try await GitWorktreeService.shared.removeWorktree(
            repoPath: repo.path,
            path: worktreePath,
            force: true,
            removalRunner: { repoPath, arguments, context, timeout in
                _ = (repoPath, arguments, context, timeout)
                try repo.removeWorktreeAdmin(named: "partial-removal-wt")
                return GitProcessResult(
                    status: 255,
                    stdout: "",
                    stdoutData: Data(),
                    stderr: "Directory not empty",
                    truncated: false
                )
            }
        )

        #expect(!FileManager.default.fileExists(atPath: worktreePath))
        let records = try await GitWorktreeService.shared.listWorktrees(repoPath: repo.path)
        #expect(!records.contains { $0.path == worktreePath })
    }

    @Test("preserves a replacement directory after Git deregisters the old worktree")
    func preservesReplacementDirectoryAfterDeregistration() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }

        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("replaced-removal-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "replaced-removal-feature",
            createBranch: true,
            baseBranch: nil
        )
        let marker = URL(fileURLWithPath: worktreePath).appendingPathComponent("replacement.txt")

        await #expect(throws: Error.self) {
            try await GitWorktreeService.shared.removeWorktree(
                repoPath: repo.path,
                path: worktreePath,
                force: true,
                removalRunner: { repoPath, arguments, context, timeout in
                    _ = try await GitProcessRunner.runGit(
                        repoPath: repoPath,
                        arguments: arguments,
                        context: context,
                        timeout: timeout
                    )
                    try FileManager.default.createDirectory(atPath: worktreePath, withIntermediateDirectories: true)
                    try "replacement".write(to: marker, atomically: true, encoding: .utf8)
                    return GitProcessResult(
                        status: 255,
                        stdout: "",
                        stdoutData: Data(),
                        stderr: "Directory not empty",
                        truncated: false
                    )
                }
            )
        }

        #expect(try String(contentsOf: marker, encoding: .utf8) == "replacement")
    }

    @Test("does not invoke Git after quiescing replaces a registered worktree")
    func doesNotInvokeGitAfterQuiescingReplacement() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }
        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("quiesce-race-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "quiesce-race-feature",
            createBranch: true,
            baseBranch: nil
        )
        let marker = URL(fileURLWithPath: worktreePath).appendingPathComponent("replacement.txt")
        let invoked = InvocationRecorder()

        await #expect(throws: WorktreeProcessQuiescerError.self) {
            try await GitWorktreeService.shared.removeWorktree(
                repoPath: repo.path,
                path: worktreePath,
                force: true,
                removalRunner: { _, _, _, _ in
                    invoked.record()
                    return GitProcessResult(status: 0, stdout: "", stdoutData: Data(), stderr: "", truncated: false)
                },
                processQuiescer: { _, _, _ in
                    try FileManager.default.removeItem(atPath: worktreePath)
                    try FileManager.default.createDirectory(atPath: worktreePath, withIntermediateDirectories: true)
                    try "replacement".write(to: marker, atomically: true, encoding: .utf8)
                }
            )
        }

        #expect(!invoked.wasInvoked)
        #expect(try String(contentsOf: marker, encoding: .utf8) == "replacement")
    }

    @Test("rejects a registered path reused by an unrelated directory")
    func rejectsRegisteredPathReusedByUnrelatedDirectory() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }
        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("preexisting-replacement-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "preexisting-replacement-feature",
            createBranch: true,
            baseBranch: nil
        )
        try FileManager.default.removeItem(atPath: worktreePath)
        try FileManager.default.createDirectory(atPath: worktreePath, withIntermediateDirectories: true)
        let marker = URL(fileURLWithPath: worktreePath).appendingPathComponent("preserved.txt")
        try "preserved".write(to: marker, atomically: true, encoding: .utf8)
        let invoked = InvocationRecorder()

        await #expect(throws: GitWorktreeService.GitWorktreeError.self) {
            try await GitWorktreeService.shared.removeWorktree(
                repoPath: repo.path,
                path: worktreePath,
                force: true,
                removalRunner: { _, _, _, _ in
                    invoked.record()
                    return GitProcessResult(status: 0, stdout: "", stdoutData: Data(), stderr: "", truncated: false)
                },
                processQuiescer: { _, _, _ in invoked.record() }
            )
        }

        #expect(!invoked.wasInvoked)
        #expect(try String(contentsOf: marker, encoding: .utf8) == "preserved")
    }

    @Test("rejects a registered path replaced by a symlink")
    func rejectsRegisteredPathReplacedBySymlink() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }
        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("symlink-replacement-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "symlink-replacement-feature",
            createBranch: true,
            baseBranch: nil
        )
        let replacement = URL(fileURLWithPath: repo.siblingPath("symlink-replacement-target"), isDirectory: true)
        try FileManager.default.createDirectory(at: replacement, withIntermediateDirectories: true)
        let marker = replacement.appendingPathComponent("preserved.txt")
        try "preserved".write(to: marker, atomically: true, encoding: .utf8)
        try FileManager.default.removeItem(atPath: worktreePath)
        try FileManager.default.createSymbolicLink(
            at: URL(fileURLWithPath: worktreePath),
            withDestinationURL: replacement
        )
        let invoked = InvocationRecorder()

        await #expect(throws: GitWorktreeService.GitWorktreeError.self) {
            try await GitWorktreeService.shared.removeWorktree(
                repoPath: repo.path,
                path: worktreePath,
                force: true,
                removalRunner: { _, _, _, _ in
                    invoked.record()
                    return GitProcessResult(status: 0, stdout: "", stdoutData: Data(), stderr: "", truncated: false)
                },
                processQuiescer: { _, _, _ in invoked.record() }
            )
        }

        #expect(!invoked.wasInvoked)
        #expect(try String(contentsOf: marker, encoding: .utf8) == "preserved")
    }

    @Test("does not quiesce a replacement during reconciliation")
    func doesNotQuiesceReplacementDuringReconciliation() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }
        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("reconciliation-race-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "reconciliation-race-feature",
            createBranch: true,
            baseBranch: nil
        )
        let quiescer = InvocationRecorder()

        await #expect(throws: Error.self) {
            try await GitWorktreeService.shared.removeWorktree(
                repoPath: repo.path,
                path: worktreePath,
                force: true,
                removalRunner: { repoPath, arguments, context, timeout in
                    _ = try await GitProcessRunner.runGit(
                        repoPath: repoPath,
                        arguments: arguments,
                        context: context,
                        timeout: timeout
                    )
                    try FileManager.default.createDirectory(atPath: worktreePath, withIntermediateDirectories: true)
                    return GitProcessResult(status: 255, stdout: "", stdoutData: Data(), stderr: "Directory not empty", truncated: false)
                },
                processQuiescer: { _, _, _ in quiescer.record() }
            )
        }

        #expect(quiescer.invocationCount == 1)
        #expect(FileManager.default.fileExists(atPath: worktreePath))
    }

    @Test("succeeds when the worktree folder is gone and git admin metadata is orphaned")
    func succeedsForOrphanedWorktree() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }

        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("orphan-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "feature",
            createBranch: true,
            baseBranch: nil
        )
        try FileManager.default.removeItem(atPath: worktreePath)
        try repo.orphanWorktreeAdmin(named: "orphan-wt")

        try await GitWorktreeService.shared.removeWorktree(repoPath: repo.path, path: worktreePath, force: true)

        let records = try await GitWorktreeService.shared.listWorktrees(repoPath: repo.path)
        #expect(!records.contains { $0.path == worktreePath })
    }

    @Test("removes registered administrative metadata when the local checkout is absent")
    func removesRegisteredMetadataWhenCheckoutIsAbsent() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }
        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("missing-checkout-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "missing-checkout-feature",
            createBranch: true,
            baseBranch: nil
        )
        try FileManager.default.removeItem(atPath: worktreePath)

        try await GitWorktreeService.shared.removeWorktree(
            repoPath: repo.path,
            path: worktreePath,
            force: true,
            processQuiescer: { _, _, _ in
                throw WorktreeProcessQuiescerError.processesStillRunning
            }
        )

        let records = try await GitWorktreeService.shared.listWorktrees(repoPath: repo.path)
        #expect(!records.contains { GitWorktreeService.canonicalPath($0.path) == GitWorktreeService.canonicalPath(worktreePath) })
    }

    @Test("throws when git leaves the worktree registered")
    func throwsWhenWorktreeSurvivesRemoval() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }

        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("locked-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "feature",
            createBranch: true,
            baseBranch: nil
        )
        try repo.run("worktree", "lock", worktreePath)

        await #expect(throws: Error.self) {
            try await GitWorktreeService.shared.removeWorktree(repoPath: repo.path, path: worktreePath, force: false)
        }
        let target = GitWorktreeService.canonicalPath(worktreePath)
        let records = try await GitWorktreeService.shared.listWorktrees(repoPath: repo.path)
        #expect(records.contains { GitWorktreeService.canonicalPath($0.path) == target })
    }

    @Test("rejects a directory that is not a registered worktree")
    func rejectsUnregisteredDirectory() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }
        let directory = repo.siblingPath("not-a-worktree")
        try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)

        await #expect(throws: Error.self) {
            try await GitWorktreeService.shared.removeWorktree(
                repoPath: repo.path,
                path: directory,
                force: true
            )
        }

        #expect(FileManager.default.fileExists(atPath: directory))
    }

    @Test("force cleanup removes an unregistered Muxy-managed checkout")
    func forceCleanupRemovesUnregisteredMuxyCheckout() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }
        let projectID = UUID()
        let worktreeRoot = MuxyFileStorage.worktreeRoot(forProjectID: projectID)
        defer {
            try? FileManager.default.removeItem(
                at: MuxyFileStorage.worktreeRoot(forProjectID: projectID, create: false)
            )
        }
        let worktreePath = worktreeRoot.appendingPathComponent("stale-worktree", isDirectory: true).path
        try FileManager.default.createDirectory(atPath: worktreePath, withIntermediateDirectories: true)
        try "stale".write(
            toFile: URL(fileURLWithPath: worktreePath).appendingPathComponent("artifact.txt").path,
            atomically: true,
            encoding: .utf8
        )
        try "gitdir: \(repo.path)/.git/worktrees/missing\n".write(
            toFile: URL(fileURLWithPath: worktreePath).appendingPathComponent(".git").path,
            atomically: true,
            encoding: .utf8
        )
        let worktree = Worktree(
            name: "stale-worktree",
            path: worktreePath,
            branch: "stale-worktree",
            source: .muxy,
            isPrimary: false
        )

        let cleanupResult = try await WorktreeStore.cleanupOnDisk(
            worktree: worktree,
            projectID: projectID,
            repoPath: repo.path,
            force: true
        )

        #expect(cleanupResult == .removed)
        #expect(!FileManager.default.fileExists(atPath: worktreePath))
    }

    @Test("force cleanup stops active processes in a stale managed checkout")
    func forceCleanupStopsActiveProcessesInStaleManagedCheckout() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }
        let projectID = UUID()
        let worktreeRoot = MuxyFileStorage.worktreeRoot(forProjectID: projectID)
        defer { try? FileManager.default.removeItem(at: worktreeRoot) }
        let worktreePath = worktreeRoot.appendingPathComponent("active-stale", isDirectory: true).path
        try FileManager.default.createDirectory(atPath: worktreePath, withIntermediateDirectories: true)
        try "gitdir: \(repo.path)/.git/worktrees/missing\n".write(
            toFile: URL(fileURLWithPath: worktreePath).appendingPathComponent(".git").path,
            atomically: true,
            encoding: .utf8
        )
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/sleep")
        process.arguments = ["30"]
        process.currentDirectoryURL = URL(fileURLWithPath: worktreePath)
        try process.run()
        defer {
            if process.isRunning {
                process.terminate()
            }
        }
        let worktree = Worktree(name: "active-stale", path: worktreePath, branch: "active-stale", source: .muxy, isPrimary: false)

        let result = try await WorktreeStore.cleanupOnDisk(
            worktree: worktree,
            projectID: projectID,
            repoPath: repo.path,
            force: true
        )

        #expect(result == .removed)
        #expect(!process.isRunning)
    }

    @Test("force cleanup succeeds when teardown removes an unregistered Muxy-managed checkout")
    func forceCleanupSucceedsWhenTeardownRemovesUnregisteredMuxyCheckout() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }
        let projectID = UUID()
        let worktreeRoot = MuxyFileStorage.worktreeRoot(forProjectID: projectID)
        defer {
            try? FileManager.default.removeItem(
                at: MuxyFileStorage.worktreeRoot(forProjectID: projectID, create: false)
            )
        }
        let worktreePath = worktreeRoot.appendingPathComponent("stale-worktree", isDirectory: true).path
        try FileManager.default.createDirectory(atPath: worktreePath, withIntermediateDirectories: true)
        try "gitdir: \(repo.path)/.git/worktrees/missing\n".write(
            toFile: URL(fileURLWithPath: worktreePath).appendingPathComponent(".git").path,
            atomically: true,
            encoding: .utf8
        )
        let configURL = URL(fileURLWithPath: repo.path, isDirectory: true)
            .appendingPathComponent(".muxy/worktree.json")
        try FileManager.default.createDirectory(
            at: configURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        let command = WorktreeConfig.SetupCommand(command: "rm -rf \"$MUXY_WORKTREE_PATH\"")
        let config = WorktreeConfig(setup: [], teardown: [command])
        try JSONEncoder().encode(config).write(to: configURL)
        let resolvedCommand = WorktreeConfig.ResolvedCommand(command: command, source: .project)
        let approval = WorktreeConfig.ProjectHookApproval(resolvedCommands: [resolvedCommand])
        let missingGlobalConfigURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("muxy-missing-global-config-\(UUID().uuidString)")
        let worktree = Worktree(
            name: "stale-worktree",
            path: worktreePath,
            branch: "stale-worktree",
            source: .muxy,
            isPrimary: false
        )

        let cleanupResult = try await WorktreeStore.cleanupOnDisk(
            worktree: worktree,
            projectID: projectID,
            repoPath: repo.path,
            projectHookApproval: approval,
            teardownGlobalConfigURL: missingGlobalConfigURL,
            force: true
        )

        #expect(cleanupResult == .removed)
        #expect(!FileManager.default.fileExists(atPath: worktreePath))
    }

    @Test("force cleanup preserves an unregistered custom checkout")
    func forceCleanupPreservesUnregisteredCustomCheckout() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }
        let worktreePath = repo.siblingPath("unregistered-custom-worktree")
        try FileManager.default.createDirectory(atPath: worktreePath, withIntermediateDirectories: true)
        let worktree = Worktree(
            name: "unregistered-custom-worktree",
            path: worktreePath,
            branch: "unregistered-custom-worktree",
            source: .muxy,
            isPrimary: false
        )

        await #expect(throws: GitWorktreeService.GitWorktreeError.self) {
            try await WorktreeStore.cleanupOnDisk(
                worktree: worktree,
                repoPath: repo.path,
                force: true
            )
        }

        #expect(FileManager.default.fileExists(atPath: worktreePath))
    }

    @Test("cleanup succeeds when an unregistered custom checkout is already absent")
    func cleanupSucceedsForAbsentCustomCheckout() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }
        let worktreePath = repo.siblingPath("absent-custom-worktree")
        let worktree = Worktree(
            name: "absent-custom-worktree",
            path: worktreePath,
            branch: "absent-custom-worktree",
            source: .muxy,
            isPrimary: false
        )

        let result = try await WorktreeStore.cleanupOnDisk(
            worktree: worktree,
            repoPath: repo.path,
            force: false
        )

        #expect(result == .removed)
    }

    @Test("cleanup forgets an unregistered remote worktree that is already absent")
    func cleanupSucceedsForAbsentRemoteWorktree() async throws {
        let resolvedPath = "/home/user/repos/removed-remote-worktree"
        let worktree = Worktree(
            name: "removed-remote-worktree",
            path: "../removed-remote-worktree",
            branch: "removed-remote-worktree",
            source: .external,
            isPrimary: false
        )
        let dependencies = WorktreeStore.CleanupDependencies(
            worktreeRegistration: { _, _, _, _ in
                GitWorktreeRegistration(path: resolvedPath, isRegistered: false)
            },
            pathExists: { path, _, _ in path != resolvedPath }
        )

        let result = try await WorktreeStore.cleanupOnDisk(
            worktree: worktree,
            repoPath: "/home/user/repos/main",
            context: .ssh(SSHDestination(host: "example.test")),
            dependencies: dependencies
        )

        #expect(result == .removed)
    }

    @Test("cleanup preserves an unregistered remote directory that still exists")
    func cleanupPreservesExistingUnregisteredRemoteDirectory() async {
        let worktree = Worktree(
            name: "unregistered-remote-worktree",
            path: "/home/user/repos/unregistered-remote-worktree",
            branch: "unregistered-remote-worktree",
            source: .external,
            isPrimary: false
        )
        let dependencies = WorktreeStore.CleanupDependencies(
            worktreeRegistration: { _, path, _, _ in
                GitWorktreeRegistration(path: path, isRegistered: false)
            },
            pathExists: { _, _, _ in true }
        )

        await #expect(throws: GitWorktreeService.GitWorktreeError.self) {
            try await WorktreeStore.cleanupOnDisk(
                worktree: worktree,
                repoPath: "/home/user/repos/main",
                context: .ssh(SSHDestination(host: "example.test")),
                dependencies: dependencies
            )
        }
    }

    @Test("force cleanup retains a reused managed checkout without Git ownership")
    func forceCleanupRetainsReusedManagedCheckout() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }
        let projectID = UUID()
        let worktreeRoot = MuxyFileStorage.worktreeRoot(forProjectID: projectID)
        defer {
            try? FileManager.default.removeItem(
                at: MuxyFileStorage.worktreeRoot(forProjectID: projectID, create: false)
            )
        }
        let worktreePath = worktreeRoot
            .appendingPathComponent("reused", isDirectory: true)
            .path
        try FileManager.default.createDirectory(atPath: worktreePath, withIntermediateDirectories: true)
        let worktree = Worktree(
            name: "reused",
            path: worktreePath,
            branch: "reused",
            source: .muxy,
            isPrimary: false
        )

        let result = try await WorktreeStore.cleanupOnDisk(
            worktree: worktree,
            projectID: projectID,
            repoPath: repo.path,
            force: true
        )

        #expect(result == .preservedUnverifiedDirectory)
        #expect(FileManager.default.fileExists(atPath: worktreePath))
    }

    @Test("force cleanup retains a managed checkout linked to another repository")
    func forceCleanupRetainsForeignManagedCheckout() async throws {
        let repo = try TempGitRepo()
        let foreignRepo = try TempGitRepo()
        defer {
            repo.cleanup()
            foreignRepo.cleanup()
        }
        let projectID = UUID()
        let worktreeRoot = MuxyFileStorage.worktreeRoot(forProjectID: projectID)
        defer {
            try? FileManager.default.removeItem(
                at: MuxyFileStorage.worktreeRoot(forProjectID: projectID, create: false)
            )
        }
        let worktreePath = worktreeRoot
            .appendingPathComponent("foreign", isDirectory: true)
            .path
        try FileManager.default.createDirectory(atPath: worktreePath, withIntermediateDirectories: true)
        try "gitdir: \(foreignRepo.path)/.git/worktrees/foreign\n".write(
            toFile: URL(fileURLWithPath: worktreePath).appendingPathComponent(".git").path,
            atomically: true,
            encoding: .utf8
        )
        let worktree = Worktree(
            name: "foreign",
            path: worktreePath,
            branch: "foreign",
            source: .muxy,
            isPrimary: false
        )

        let result = try await WorktreeStore.cleanupOnDisk(
            worktree: worktree,
            projectID: projectID,
            repoPath: repo.path,
            force: true
        )

        #expect(result == .preservedUnverifiedDirectory)
        #expect(FileManager.default.fileExists(atPath: worktreePath))
    }

    @Test("force cleanup preserves another project's managed checkout")
    func forceCleanupPreservesAnotherProjectsCheckout() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }
        let owningProjectID = UUID()
        let otherProjectID = UUID()
        let worktreePath = MuxyFileStorage.worktreeRoot(forProjectID: otherProjectID)
            .appendingPathComponent("other-project-worktree", isDirectory: true)
            .path
        defer {
            try? FileManager.default.removeItem(
                at: MuxyFileStorage.worktreeRoot(forProjectID: otherProjectID, create: false)
            )
        }
        try FileManager.default.createDirectory(atPath: worktreePath, withIntermediateDirectories: true)
        let worktree = Worktree(
            name: "other-project-worktree",
            path: worktreePath,
            branch: "other-project-worktree",
            source: .muxy,
            isPrimary: false
        )

        await #expect(throws: GitWorktreeService.GitWorktreeError.self) {
            try await WorktreeStore.cleanupOnDisk(
                worktree: worktree,
                projectID: owningProjectID,
                repoPath: repo.path,
                force: true
            )
        }

        #expect(FileManager.default.fileExists(atPath: worktreePath))
    }

    @Test("force cleanup preserves a checkout beneath a symlinked project root")
    func forceCleanupPreservesCheckoutUnderSymlinkedProjectRoot() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }
        let projectID = UUID()
        let expectedRoot = MuxyFileStorage.worktreeRoot(forProjectID: projectID)
        try FileManager.default.removeItem(at: expectedRoot)
        let outsideRoot = FileManager.default.temporaryDirectory
            .appendingPathComponent("muxy-unrelated-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: outsideRoot, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: outsideRoot) }
        try FileManager.default.createSymbolicLink(at: expectedRoot, withDestinationURL: outsideRoot)
        defer {
            try? FileManager.default.removeItem(
                at: MuxyFileStorage.worktreeRoot(forProjectID: projectID, create: false)
            )
        }
        let worktreePath = expectedRoot.appendingPathComponent("unrelated", isDirectory: true).path
        try FileManager.default.createDirectory(atPath: worktreePath, withIntermediateDirectories: true)
        let worktree = Worktree(
            name: "unrelated",
            path: worktreePath,
            branch: "unrelated",
            source: .muxy,
            isPrimary: false
        )

        await #expect(throws: GitWorktreeService.GitWorktreeError.self) {
            try await WorktreeStore.cleanupOnDisk(
                worktree: worktree,
                projectID: projectID,
                repoPath: repo.path,
                force: true
            )
        }

        #expect(FileManager.default.fileExists(atPath: worktreePath))
    }

    @Test("cleanupOnDisk removes the worktree but keeps its branch")
    func cleanupKeepsBranch() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }

        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("keep-branch-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "feature",
            createBranch: true,
            baseBranch: nil
        )

        let worktree = Worktree(name: "keep-branch-wt", path: worktreePath, branch: "feature", isPrimary: false)
        let cleanupResult = try await WorktreeStore.cleanupOnDisk(worktree: worktree, repoPath: repo.path)

        let records = try await GitWorktreeService.shared.listWorktrees(repoPath: repo.path)
        #expect(!records.contains { $0.path == worktreePath })
        #expect(repo.branchExists("feature"))
        #expect(cleanupResult == .removed)
    }

    @Test("cleanupOnDisk without force preserves a dirty worktree")
    func cleanupWithoutForcePreservesDirtyWorktree() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }

        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("dirty-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "dirty-feature",
            createBranch: true,
            baseBranch: nil
        )
        try "dirty".write(
            toFile: URL(fileURLWithPath: worktreePath).appendingPathComponent("a.txt").path,
            atomically: true,
            encoding: .utf8
        )
        let worktree = Worktree(name: "dirty-wt", path: worktreePath, branch: "dirty-feature", isPrimary: false)

        await #expect(throws: Error.self) {
            try await WorktreeStore.cleanupOnDisk(
                worktree: worktree,
                repoPath: repo.path,
                force: false
            )
        }

        #expect(FileManager.default.fileExists(atPath: worktreePath))
        let records = try await GitWorktreeService.shared.listWorktrees(repoPath: repo.path)
        let target = GitWorktreeService.canonicalPath(worktreePath)
        #expect(records.contains { GitWorktreeService.canonicalPath($0.path) == target })
    }

    @Test("uncommitted change inspection surfaces git failures")
    func uncommittedChangeInspectionSurfacesGitFailures() async {
        await #expect(throws: Error.self) {
            try await GitWorktreeService.shared.uncommittedChanges(
                worktreePath: "/tmp/muxy-missing-worktree-\(UUID().uuidString)"
            )
        }
    }

    @Test("a timeout after git deregisters the worktree is reconciled as success")
    func reconcilesTimeoutAfterDeregistration() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }

        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("timeout-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "timeout-feature",
            createBranch: true,
            baseBranch: nil
        )

        try await GitWorktreeService.shared.removeWorktree(
            repoPath: repo.path,
            path: worktreePath,
            force: true,
            removalRunner: { repoPath, arguments, context, timeout in
                _ = try await GitProcessRunner.runGit(
                    repoPath: repoPath,
                    arguments: arguments,
                    context: context,
                    timeout: timeout
                )
                throw GitProcessError.timedOut(timeout)
            }
        )

        let records = try await GitWorktreeService.shared.listWorktrees(repoPath: repo.path)
        let target = GitWorktreeService.canonicalPath(worktreePath)
        #expect(!records.contains { GitWorktreeService.canonicalPath($0.path) == target })
    }

    @Test("reconciles after the removal deadline has elapsed")
    func reconcilesAfterRemovalDeadlineElapsed() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }

        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("timeout-floor-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "timeout-floor-feature",
            createBranch: true,
            baseBranch: nil
        )

        try await GitWorktreeService.shared.removeWorktree(
            repoPath: repo.path,
            path: worktreePath,
            force: true,
            timeout: 5,
            removalRunner: { repoPath, arguments, context, _ in
                _ = try await GitProcessRunner.runGit(
                    repoPath: repoPath,
                    arguments: arguments,
                    context: context,
                    timeout: 10
                )
                try await Task.sleep(for: .milliseconds(5100))
                return GitProcessResult(status: 0, stdout: "", stdoutData: Data(), stderr: "", truncated: false)
            }
        )

        let records = try await GitWorktreeService.shared.listWorktrees(repoPath: repo.path)
        #expect(!records.contains { GitWorktreeService.canonicalPath($0.path) == GitWorktreeService.canonicalPath(worktreePath) })
    }

    @Test("cleanupOnDisk preserves an orphaned worktree when its main repo is missing")
    func cleanupPreservesOrphanedWorktreeWhenRepoIsMissing() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }

        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("orphan-repo-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "feature",
            createBranch: true,
            baseBranch: nil
        )

        try FileManager.default.removeItem(atPath: repo.path)

        let worktree = Worktree(name: "orphan-repo-wt", path: worktreePath, branch: "feature", isPrimary: false)
        let cleanupResult = try await WorktreeStore.cleanupOnDisk(worktree: worktree, repoPath: repo.path)

        #expect(cleanupResult == .preservedMissingRepository)
        #expect(FileManager.default.fileExists(atPath: worktreePath))
    }

    @Test("cleanupOnDisk preserves a reused path when its main repo is missing")
    func cleanupPreservesReusedPathWhenRepoIsMissing() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }

        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("reused-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "feature",
            createBranch: true,
            baseBranch: nil
        )
        try FileManager.default.removeItem(atPath: repo.path)
        try FileManager.default.removeItem(atPath: worktreePath)
        try FileManager.default.createDirectory(atPath: worktreePath, withIntermediateDirectories: true)
        let preservedFile = URL(fileURLWithPath: worktreePath).appendingPathComponent("preserve.txt")
        try "unrelated".write(to: preservedFile, atomically: true, encoding: .utf8)

        let worktree = Worktree(name: "reused-wt", path: worktreePath, branch: "feature", isPrimary: false)
        let cleanupResult = try await WorktreeStore.cleanupOnDisk(worktree: worktree, repoPath: repo.path)

        #expect(cleanupResult == .preservedMissingRepository)
        #expect(try String(contentsOf: preservedFile, encoding: .utf8) == "unrelated")
    }

    @Test("cleanupOnDisk without force preserves an orphan when its main repo is missing")
    func cleanupWithoutForcePreservesOrphanWhenRepoIsMissing() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }

        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("dirty-orphan-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "feature",
            createBranch: true,
            baseBranch: nil
        )
        let changedFile = URL(fileURLWithPath: worktreePath).appendingPathComponent("a.txt")
        try "dirty".write(to: changedFile, atomically: true, encoding: .utf8)
        try FileManager.default.removeItem(atPath: repo.path)

        let worktree = Worktree(name: "dirty-orphan-wt", path: worktreePath, branch: "feature", isPrimary: false)
        let cleanupResult = try await WorktreeStore.cleanupOnDisk(
            worktree: worktree,
            repoPath: repo.path,
            force: false
        )

        #expect(cleanupResult == .preservedMissingRepository)
        #expect(try String(contentsOf: changedFile, encoding: .utf8) == "dirty")
    }

    @Test("project cleanup preserves worktrees when the main repo is missing")
    func projectCleanupPreservesWorktreesWhenRepoIsMissing() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }

        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let project = Project(name: "Repo", path: repo.path)
        let worktreeRoot = MuxyFileStorage.worktreeRoot(forProjectID: project.id)
        let worktreePath = worktreeRoot.appendingPathComponent("project-orphan-wt").path
        defer {
            try? FileManager.default.removeItem(
                at: MuxyFileStorage.worktreeRoot(forProjectID: project.id, create: false)
            )
        }
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "feature",
            createBranch: true,
            baseBranch: nil
        )
        let worktree = Worktree(name: "project-orphan-wt", path: worktreePath, branch: "feature", isPrimary: false)
        try FileManager.default.removeItem(atPath: repo.path)

        try await WorktreeStore.cleanupOnDisk(for: project, knownWorktrees: [worktree])

        #expect(FileManager.default.fileExists(atPath: worktreePath))
    }

    @Test("project cleanup preserves unknown directories in its managed root")
    func projectCleanupPreservesUnknownManagedDirectories() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }
        let project = Project(name: "Repo", path: repo.path)
        let root = MuxyFileStorage.worktreeRoot(forProjectID: project.id)
        let unknownPath = root.appendingPathComponent("unknown", isDirectory: true)
        try FileManager.default.createDirectory(at: unknownPath, withIntermediateDirectories: true)
        defer {
            try? FileManager.default.removeItem(
                at: MuxyFileStorage.worktreeRoot(forProjectID: project.id, create: false)
            )
        }

        try await WorktreeStore.cleanupOnDisk(for: project, knownWorktrees: [])

        #expect(FileManager.default.fileExists(atPath: unknownPath.path))
    }

    @Test("project cleanup preserves quarantined replacement directories")
    func projectCleanupPreservesQuarantinedReplacementDirectories() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }
        let project = Project(name: "Repo", path: repo.path)
        let worktreeRoot = MuxyFileStorage.worktreeRoot(forProjectID: project.id)
        defer { try? FileManager.default.removeItem(at: worktreeRoot) }
        let quarantine = worktreeRoot.appendingPathComponent(
            "\(WorktreeProcessQuiescer.quarantinePrefix)replacement",
            isDirectory: true
        )
        try FileManager.default.createDirectory(at: quarantine, withIntermediateDirectories: true)
        let marker = quarantine.appendingPathComponent("preserved.txt")
        try "preserved".write(to: marker, atomically: true, encoding: .utf8)

        try await WorktreeStore.cleanupOnDisk(for: project, knownWorktrees: [])

        #expect(try String(contentsOf: marker, encoding: .utf8) == "preserved")
    }

    @Test("heals an orphaned worktree referenced through a symlinked parent")
    func healsOrphanThroughSymlinkedParent() async throws {
        let repo = try TempGitRepo()
        defer { repo.cleanup() }

        try repo.commit(file: "a.txt", contents: "1", message: "base")
        let worktreePath = repo.siblingPath("symlink-wt")
        try await GitWorktreeService.shared.addWorktree(
            repoPath: repo.path,
            path: worktreePath,
            branch: "feature",
            createBranch: true,
            baseBranch: nil
        )
        try FileManager.default.removeItem(atPath: worktreePath)
        try repo.orphanWorktreeAdmin(named: "symlink-wt")

        let aliasPath = try repo.symlinkedSiblingPath(for: "symlink-wt")
        try await GitWorktreeService.shared.removeWorktree(repoPath: repo.path, path: aliasPath, force: true)

        let records = try await GitWorktreeService.shared.listWorktrees(repoPath: repo.path)
        #expect(!records.contains { GitWorktreeService.canonicalPath($0.path) == GitWorktreeService.canonicalPath(worktreePath) })
    }

    @Test("local presence checks include dangling symlinks")
    func localPresenceIncludesDanglingSymlinks() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("muxy-presence-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let link = root.appendingPathComponent("dangling")
        try FileManager.default.createSymbolicLink(
            at: link,
            withDestinationURL: root.appendingPathComponent("missing")
        )

        #expect(await LocalFileOps().exists(at: link.path))
        #expect(try await LocalFileOps().exists(at: link.path, timeout: 1))
        #expect(try await !LocalFileOps().exists(at: root.appendingPathComponent("absent").path, timeout: 1))

        let restricted = root.appendingPathComponent("restricted", isDirectory: true)
        try FileManager.default.createDirectory(at: restricted, withIntermediateDirectories: true)
        try FileManager.default.setAttributes([.posixPermissions: 0], ofItemAtPath: restricted.path)
        defer { try? FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: restricted.path) }
        #expect(try await LocalFileOps().exists(at: restricted.appendingPathComponent("unknown").path, timeout: 1))
    }
}

private final class InvocationRecorder: @unchecked Sendable {
    private(set) var invocationCount = 0

    var wasInvoked: Bool {
        invocationCount > 0
    }

    func record() {
        invocationCount += 1
    }
}

private struct TempGitRepo {
    let path: String
    private let parent: String

    init() throws {
        let base = FileManager.default.temporaryDirectory
            .appendingPathComponent("muxy-worktree-remove-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
        parent = base.path
        path = base.appendingPathComponent("repo", isDirectory: true).path
        try FileManager.default.createDirectory(atPath: path, withIntermediateDirectories: true)
        try run("init", "-q", "-b", "main")
        try run("config", "user.email", "test@example.com")
        try run("config", "user.name", "Test")
        try run("config", "commit.gpgsign", "false")
    }

    func cleanup() {
        try? FileManager.default.removeItem(atPath: parent)
    }

    func siblingPath(_ name: String) -> String {
        URL(fileURLWithPath: parent).appendingPathComponent(name).path
    }

    func commit(file: String, contents: String, message: String) throws {
        let fileURL = URL(fileURLWithPath: path).appendingPathComponent(file)
        try contents.write(to: fileURL, atomically: true, encoding: .utf8)
        try run("add", file)
        try run("commit", "-q", "-m", message)
    }

    func symlinkedSiblingPath(for name: String) throws -> String {
        let realParent = URL(fileURLWithPath: parent)
        let aliasParent = realParent.appendingPathComponent("alias-\(UUID().uuidString)")
        try FileManager.default.createSymbolicLink(at: aliasParent, withDestinationURL: realParent)
        return aliasParent.appendingPathComponent(name).path
    }

    func branchExists(_ branch: String) -> Bool {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = ["git", "-C", path, "branch", "--list", "--format=%(refname:short)"]
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = Pipe()
        guard (try? process.run()) != nil else { return false }
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        let output = String(data: data, encoding: .utf8) ?? ""
        return output.split(separator: "\n").map { $0.trimmingCharacters(in: .whitespaces) }.contains(branch)
    }

    func orphanWorktreeAdmin(named name: String) throws {
        let gitdir = URL(fileURLWithPath: path)
            .appendingPathComponent(".git/worktrees/\(name)/gitdir")
        try "/nonexistent/\(name)/.git\n".write(to: gitdir, atomically: true, encoding: .utf8)
    }

    func removeWorktreeAdmin(named name: String) throws {
        try FileManager.default.removeItem(
            at: URL(fileURLWithPath: path).appendingPathComponent(".git/worktrees/\(name)")
        )
    }

    func run(_ args: String...) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = ["git", "-C", path] + args
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = pipe
        try process.run()
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            throw NSError(
                domain: "GitTestRepo",
                code: Int(process.terminationStatus),
                userInfo: [NSLocalizedDescriptionKey: String(data: data, encoding: .utf8) ?? ""]
            )
        }
    }
}
