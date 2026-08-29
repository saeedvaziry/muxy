import Foundation

struct GitWorktreeRecord: Hashable {
    let path: String
    let branch: String?
    let head: String?
    let isBare: Bool
    let isDetached: Bool
    let isPrunable: Bool

    init(
        path: String,
        branch: String?,
        head: String?,
        isBare: Bool,
        isDetached: Bool,
        isPrunable: Bool = false
    ) {
        self.path = path
        self.branch = branch
        self.head = head
        self.isBare = isBare
        self.isDetached = isDetached
        self.isPrunable = isPrunable
    }
}

struct GitWorktreeRegistration: Equatable, Sendable {
    let path: String
    let isRegistered: Bool
}

protocol GitWorktreeListing {
    func listWorktrees(repoPath: String) async throws -> [GitWorktreeRecord]
}

actor GitWorktreeService: GitWorktreeListing {
    typealias RemovalRunner = @Sendable (
        _ repoPath: String,
        _ arguments: [String],
        _ context: WorkspaceContext,
        _ timeout: TimeInterval
    ) async throws -> GitProcessResult
    typealias ProcessQuiescer = @Sendable (
        _ path: String,
        _ identity: WorktreeProcessQuiescer.DirectoryIdentity,
        _ timeout: TimeInterval
    ) async throws -> Void

    private struct RemovalTarget {
        let canonicalPath: String
        let residualPath: String
        let repoPath: String
        let context: WorkspaceContext
        let wasRegistered: Bool
        let originalDirectoryIdentity: WorktreeProcessQuiescer.DirectoryIdentity?
    }

    static let shared = GitWorktreeService()
    static let defaultWorktreeRemovalTimeout: TimeInterval = 300
    private static let removalReconciliationTimeout: TimeInterval = 5
    private static let maxConcurrentRepositoryChecksPerContext = 4
    private static let localRepositoryCheckTimeout = Duration.seconds(10)

    private let repositoryCheckCoordinator: GitRepositoryCheckCoordinator

    enum GitWorktreeError: LocalizedError {
        case notGitRepository
        case notRegistered
        case commandFailed(String)

        var errorDescription: String? {
            switch self {
            case .notGitRepository:
                "This folder is not a Git repository."
            case .notRegistered:
                "Worktree is not registered with this repository."
            case let .commandFailed(message):
                message
            }
        }
    }

    private init() {
        repositoryCheckCoordinator = GitRepositoryCheckCoordinator(
            maxConcurrentChecksPerContext: Self.maxConcurrentRepositoryChecksPerContext
        ) { path, context in
            await GitWorktreeService.probeGitRepository(path: path, context: context)
        }
    }

    func isGitRepository(_ path: String, context: WorkspaceContext = .local) async -> Bool {
        await repositoryCheckCoordinator.isGitRepository(path, context: context)
    }

    private static func probeGitRepository(path: String, context: WorkspaceContext) async -> Bool {
        guard case .local = context else {
            return await runRepositoryProbe(path: path, context: context)
        }
        return await withTaskGroup(of: Bool.self, returning: Bool.self) { group in
            group.addTask { await runRepositoryProbe(path: path, context: .local) }
            group.addTask {
                try? await Task.sleep(for: localRepositoryCheckTimeout)
                return false
            }
            let result = await group.next() ?? false
            group.cancelAll()
            return result
        }
    }

    private static func runRepositoryProbe(path: String, context: WorkspaceContext) async -> Bool {
        guard let result = try? await GitProcessRunner.runGit(
            repoPath: path,
            arguments: ["rev-parse", "--is-inside-work-tree"],
            context: context
        )
        else {
            return false
        }
        return result.status == 0 &&
            result.stdout.trimmingCharacters(in: .whitespacesAndNewlines) == "true"
    }

    func hasUncommittedChanges(worktreePath: String, context: WorkspaceContext = .local) async -> Bool {
        await (try? uncommittedChanges(worktreePath: worktreePath, context: context)) ?? false
    }

    func uncommittedChanges(
        worktreePath: String,
        context: WorkspaceContext = .local,
        timeout: TimeInterval? = nil
    ) async throws -> Bool {
        let result = try await GitProcessRunner.runGit(
            repoPath: worktreePath,
            arguments: ["status", "--porcelain=1", "--untracked-files=all"],
            context: context,
            timeout: timeout
        )
        guard result.status == 0 else {
            throw GitWorktreeError.commandFailed(
                result.stderr.isEmpty ? "Failed to inspect worktree changes." : result.stderr
            )
        }
        return !result.stdout.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    func listWorktrees(repoPath: String) async throws -> [GitWorktreeRecord] {
        try await listWorktrees(repoPath: repoPath, context: .local)
    }

    func listWorktrees(repoPath: String, context: WorkspaceContext) async throws -> [GitWorktreeRecord] {
        let result = try await GitProcessRunner.runGit(
            repoPath: repoPath,
            arguments: ["worktree", "list", "--porcelain"],
            context: context
        )
        guard result.status == 0 else {
            throw GitWorktreeError.commandFailed(
                result.stderr.isEmpty ? "Failed to list worktrees." : result.stderr
            )
        }
        return parsePorcelain(result.stdout)
    }

    static let allowedBranchCharacters = CharacterSet.alphanumerics
        .union(CharacterSet(charactersIn: "._/-"))

    private static func validateBranchName(_ branch: String) throws {
        guard !branch.isEmpty,
              !branch.hasPrefix("-"),
              branch.unicodeScalars.allSatisfy({ Self.allowedBranchCharacters.contains($0) })
        else {
            throw GitWorktreeError.commandFailed("Invalid branch name.")
        }
    }

    func addWorktree(
        repoPath: String,
        path: String,
        branch: String,
        createBranch: Bool,
        baseBranch: String? = nil,
        context: WorkspaceContext = .local
    ) async throws {
        try Self.validateBranchName(branch)
        var args: [String] = ["worktree", "add"]
        if createBranch {
            args += ["-b", branch, path]
            if let baseBranch {
                try Self.validateBranchName(baseBranch)
                args.append(baseBranch)
            }
        } else {
            args += ["--", path, branch]
        }
        let result = try await GitProcessRunner.runGit(repoPath: repoPath, arguments: args, context: context)
        guard result.status == 0 else {
            throw GitWorktreeError.commandFailed(
                result.stderr.isEmpty ? "Failed to add worktree." : result.stderr
            )
        }
    }

    @discardableResult
    func removeWorktree(
        repoPath: String,
        path: String,
        force: Bool = false,
        context: WorkspaceContext = .local,
        timeout: TimeInterval = defaultWorktreeRemovalTimeout,
        removalRunner: RemovalRunner = runRemoval,
        processQuiescer: ProcessQuiescer = { path, identity, timeout in
            try await WorktreeProcessQuiescer.quiesce(path: path, matching: identity, timeout: timeout)
        }
    ) async throws -> String {
        let deadline = OperationDeadline(timeout: timeout)
        let records = try await listWorktrees(
            repoPath: repoPath,
            context: context,
            timeout: deadline.remaining()
        )
        let resolutions = try await WorkspacePathResolver.live.resolve(
            paths: [path] + records.map(\.path),
            relativeTo: repoPath,
            context: context,
            timeout: deadline.remaining()
        )
        guard let removalPath = resolutions.first?.path else {
            throw GitWorktreeError.commandFailed("Failed to resolve the worktree path.")
        }
        let target = Self.canonicalPath(removalPath, context: context)
        let wasRegistered = resolutions.dropFirst().contains {
            Self.canonicalPath($0.path, context: context) == target
        }
        let primaryTarget = resolutions.dropFirst().first.map {
            Self.canonicalPath($0.path, context: context)
        }
        if target == primaryTarget {
            throw GitWorktreeError.commandFailed("The primary worktree cannot be removed.")
        }
        if !wasRegistered, try await context.fileOps.exists(at: removalPath, timeout: deadline.remaining()) {
            throw GitWorktreeError.notRegistered
        }
        let residualPath = removalPath
        let originalDirectoryIdentity = context.isRemote
            ? nil
            : WorktreeProcessQuiescer.directoryIdentity(at: residualPath)
        let removalTarget = RemovalTarget(
            canonicalPath: target,
            residualPath: residualPath,
            repoPath: repoPath,
            context: context,
            wasRegistered: wasRegistered,
            originalDirectoryIdentity: originalDirectoryIdentity
        )
        if wasRegistered, !context.isRemote {
            if try await context.fileOps.exists(at: residualPath, timeout: deadline.remaining()) {
                guard Self.isOwnedLocalCheckout(
                    originalPath: path,
                    checkoutPath: residualPath,
                    repoPath: repoPath
                )
                else {
                    throw GitWorktreeError.commandFailed("Worktree ownership could not be verified.")
                }
                guard let originalDirectoryIdentity else {
                    throw WorktreeProcessQuiescerError.directoryChanged(recoveryPath: nil)
                }
                try await processQuiescer(residualPath, originalDirectoryIdentity, deadline.remaining())
                guard WorktreeProcessQuiescer.directoryIdentity(at: residualPath) == originalDirectoryIdentity else {
                    throw WorktreeProcessQuiescerError.directoryChanged(recoveryPath: nil)
                }
            }
        }

        var args: [String] = ["worktree", "remove"]
        if force {
            args.append("--force")
        }
        args += ["--", removalPath]
        let result: GitProcessResult
        do {
            result = try await removalRunner(repoPath, args, context, deadline.remaining())
        } catch {
            guard Self.isTimeout(error) else { throw error }
            try await reconcileRemoval(
                removalTarget,
                processQuiescer: processQuiescer,
                timeout: Self.removalReconciliationTimeout,
                failureMessage: error.localizedDescription
            )
            return removalPath
        }
        if result.status != 0, let remaining = try? deadline.remaining() {
            try? await pruneWorktrees(repoPath: repoPath, context: context, timeout: remaining)
        }
        try Task.checkCancellation()
        let verificationTimeout = max(
            (try? deadline.remaining()) ?? 0,
            Self.removalReconciliationTimeout
        )
        try await reconcileRemoval(
            removalTarget,
            processQuiescer: processQuiescer,
            timeout: verificationTimeout,
            failureMessage: result.stderr.isEmpty ? "Failed to remove worktree." : result.stderr
        )
        return removalPath
    }

    private func reconcileRemoval(
        _ target: RemovalTarget,
        processQuiescer: ProcessQuiescer,
        timeout: TimeInterval,
        failureMessage: String
    ) async throws {
        let deadline = OperationDeadline(timeout: timeout)
        let stillRegistered = try await isRegistered(
            target: target.canonicalPath,
            repoPath: target.repoPath,
            context: target.context,
            timeout: deadline.remaining()
        )
        guard !stillRegistered else {
            throw GitWorktreeError.commandFailed(failureMessage)
        }

        guard try await target.context.fileOps.exists(
            at: target.residualPath,
            timeout: deadline.remaining()
        )
        else { return }
        guard target.wasRegistered else {
            throw GitWorktreeError.commandFailed(failureMessage)
        }
        guard !target.context.isRemote, let originalDirectoryIdentity = target.originalDirectoryIdentity else {
            throw GitWorktreeError.commandFailed("\(failureMessage) The remaining directory could not be verified.")
        }
        guard WorktreeProcessQuiescer.directoryIdentity(at: target.residualPath) == originalDirectoryIdentity else {
            throw GitWorktreeError.commandFailed("\(failureMessage) The remaining directory changed during removal.")
        }

        do {
            try await processQuiescer(target.residualPath, originalDirectoryIdentity, deadline.remaining())
            guard WorktreeProcessQuiescer.directoryIdentity(at: target.residualPath) == originalDirectoryIdentity else {
                throw WorktreeProcessQuiescerError.directoryChanged(recoveryPath: nil)
            }
            try await WorktreeProcessQuiescer.removeDirectory(
                at: target.residualPath,
                matching: originalDirectoryIdentity,
                timeout: deadline.remaining()
            )
        } catch {
            throw GitWorktreeError.commandFailed("\(failureMessage) Residual cleanup failed: \(error.localizedDescription)")
        }
        guard try await !target.context.fileOps.exists(
            at: target.residualPath,
            timeout: deadline.remaining()
        )
        else {
            throw GitWorktreeError.commandFailed("\(failureMessage) The worktree directory still exists.")
        }
    }

    func isWorktreeRegistered(
        repoPath: String,
        path: String,
        context: WorkspaceContext = .local,
        timeout: TimeInterval = defaultWorktreeRemovalTimeout
    ) async throws -> Bool {
        try await worktreeRegistration(
            repoPath: repoPath,
            path: path,
            context: context,
            timeout: timeout
        ).isRegistered
    }

    func worktreeRegistration(
        repoPath: String,
        path: String,
        context: WorkspaceContext = .local,
        timeout: TimeInterval = defaultWorktreeRemovalTimeout
    ) async throws -> GitWorktreeRegistration {
        let deadline = OperationDeadline(timeout: timeout)
        let records = try await listWorktrees(
            repoPath: repoPath,
            context: context,
            timeout: deadline.remaining()
        )
        let resolutions = try await WorkspacePathResolver.live.resolve(
            paths: [path] + records.map(\.path),
            relativeTo: repoPath,
            context: context,
            timeout: deadline.remaining()
        )
        guard let resolvedPath = resolutions.first?.path else {
            throw WorkspacePathResolverError.invalidOutput
        }
        let target = Self.canonicalPath(resolvedPath, context: context)
        let isRegistered = resolutions.dropFirst().contains {
            Self.canonicalPath($0.path, context: context) == target
        }
        return GitWorktreeRegistration(path: resolvedPath, isRegistered: isRegistered)
    }

    private func pruneWorktrees(repoPath: String, context: WorkspaceContext, timeout: TimeInterval) async throws {
        let result = try await GitProcessRunner.runGit(
            repoPath: repoPath,
            arguments: ["worktree", "prune"],
            context: context,
            timeout: timeout
        )
        guard result.status == 0 else {
            throw GitWorktreeError.commandFailed(
                result.stderr.isEmpty ? "Failed to prune worktrees." : result.stderr
            )
        }
    }

    private func isRegistered(
        target: String,
        repoPath: String,
        context: WorkspaceContext,
        timeout: TimeInterval
    ) async throws -> Bool {
        let deadline = OperationDeadline(timeout: timeout)
        let records = try await listWorktrees(
            repoPath: repoPath,
            context: context,
            timeout: deadline.remaining()
        )
        let resolutions = try await WorkspacePathResolver.live.resolve(
            paths: records.map(\.path),
            relativeTo: repoPath,
            context: context,
            timeout: deadline.remaining()
        )
        return resolutions.contains { Self.canonicalPath($0.path, context: context) == target }
    }

    private static func runRemoval(
        repoPath: String,
        arguments: [String],
        context: WorkspaceContext,
        timeout: TimeInterval
    ) async throws -> GitProcessResult {
        try await GitProcessRunner.runGit(
            repoPath: repoPath,
            arguments: arguments,
            context: context,
            timeout: timeout
        )
    }

    private static func isTimeout(_ error: Error) -> Bool {
        if let error = error as? GitProcessError, case .timedOut = error {
            return true
        }
        if let error = error as? SSHCommandError, case .timedOut = error {
            return true
        }
        return false
    }

    private static func isOwnedLocalCheckout(originalPath: String, checkoutPath: String, repoPath: String) -> Bool {
        let inputPath = localInputPath(originalPath, relativeTo: repoPath)
        guard (try? FileManager.default.destinationOfSymbolicLink(atPath: inputPath)) == nil else { return false }
        let checkout = URL(fileURLWithPath: checkoutPath, isDirectory: true).standardizedFileURL
        let checkoutGit = checkout.appendingPathComponent(".git")
        guard (try? FileManager.default.destinationOfSymbolicLink(atPath: checkoutGit.path)) == nil,
              let attributes = try? FileManager.default.attributesOfItem(atPath: checkoutGit.path),
              attributes[.type] as? FileAttributeType == .typeRegular,
              let size = attributes[.size] as? NSNumber,
              size.intValue <= 4096,
              let gitdir = gitDirectoryReference(at: checkoutGit)
        else { return false }
        guard let commonGitDirectory = commonGitDirectory(repoPath: repoPath) else { return false }
        let worktrees = commonGitDirectory.appendingPathComponent("worktrees", isDirectory: true)
        guard gitdir.deletingLastPathComponent() == worktrees else { return false }
        let adminGitdir = gitdir.appendingPathComponent("gitdir")
        guard (try? FileManager.default.destinationOfSymbolicLink(atPath: adminGitdir.path)) == nil,
              let backlink = adminGitdirBacklink(at: adminGitdir),
              backlink == checkoutGit.resolvingSymlinksInPath().standardizedFileURL
        else { return false }
        return true
    }

    private static func commonGitDirectory(repoPath: String) -> URL? {
        let dotGit = URL(fileURLWithPath: repoPath, isDirectory: true).appendingPathComponent(".git")
        guard (try? FileManager.default.destinationOfSymbolicLink(atPath: dotGit.path)) == nil else { return nil }
        if (try? dotGit.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true {
            return dotGit.resolvingSymlinksInPath().standardizedFileURL
        }
        guard let gitdir = gitDirectoryReference(at: dotGit) else { return nil }
        if gitdir.deletingLastPathComponent().lastPathComponent == "worktrees" {
            return gitdir.deletingLastPathComponent().deletingLastPathComponent()
        }
        return gitdir
    }

    private static func localInputPath(_ path: String, relativeTo repoPath: String) -> String {
        let expanded = NSString(string: path).expandingTildeInPath
        guard !expanded.hasPrefix("/") else {
            return URL(fileURLWithPath: expanded).standardizedFileURL.path
        }
        return URL(fileURLWithPath: repoPath, isDirectory: true)
            .appendingPathComponent(expanded)
            .standardizedFileURL.path
    }

    private static func gitDirectoryReference(at file: URL) -> URL? {
        guard let contents = try? String(contentsOf: file, encoding: .utf8),
              let firstLine = contents.split(whereSeparator: \Character.isNewline).first,
              firstLine.hasPrefix("gitdir: ")
        else { return nil }
        let reference = firstLine.dropFirst("gitdir: ".count).trimmingCharacters(in: .whitespaces)
        guard !reference.isEmpty else { return nil }
        return URL(fileURLWithPath: reference, relativeTo: file.deletingLastPathComponent())
            .standardizedFileURL
            .resolvingSymlinksInPath()
            .standardizedFileURL
    }

    private static func adminGitdirBacklink(at file: URL) -> URL? {
        guard let contents = try? String(contentsOf: file, encoding: .utf8),
              let firstLine = contents.split(whereSeparator: \Character.isNewline).first
        else { return nil }
        let reference = firstLine.trimmingCharacters(in: .whitespaces)
        guard !reference.isEmpty else { return nil }
        return URL(fileURLWithPath: reference, relativeTo: file.deletingLastPathComponent())
            .standardizedFileURL
            .resolvingSymlinksInPath()
            .standardizedFileURL
    }

    private func listWorktrees(
        repoPath: String,
        context: WorkspaceContext,
        timeout: TimeInterval
    ) async throws -> [GitWorktreeRecord] {
        let result = try await GitProcessRunner.runGit(
            repoPath: repoPath,
            arguments: ["worktree", "list", "--porcelain"],
            context: context,
            timeout: timeout
        )
        guard result.status == 0 else {
            throw GitWorktreeError.commandFailed(
                result.stderr.isEmpty ? "Failed to list worktrees." : result.stderr
            )
        }
        return parsePorcelain(result.stdout)
    }

    static func canonicalPath(_ path: String, context: WorkspaceContext = .local) -> String {
        guard !context.isRemote else { return ProjectPickerPathService.standardizedRemotePath(path) }
        let standardized = URL(fileURLWithPath: path).standardizedFileURL
        let resolved = standardized.resolvingSymlinksInPath()
        guard resolved.path == standardized.path else { return resolved.path }

        let parent = standardized.deletingLastPathComponent().resolvingSymlinksInPath()
        return parent.appendingPathComponent(standardized.lastPathComponent).path
    }

    private func parsePorcelain(_ raw: String) -> [GitWorktreeRecord] {
        var records: [GitWorktreeRecord] = []
        var currentPath: String?
        var currentBranch: String?
        var currentHead: String?
        var isBare = false
        var isDetached = false
        var isPrunable = false

        func flush() {
            guard let path = currentPath else { return }
            records.append(GitWorktreeRecord(
                path: path,
                branch: currentBranch,
                head: currentHead,
                isBare: isBare,
                isDetached: isDetached,
                isPrunable: isPrunable
            ))
            currentPath = nil
            currentBranch = nil
            currentHead = nil
            isBare = false
            isDetached = false
            isPrunable = false
        }

        for line in raw.split(separator: "\n", omittingEmptySubsequences: false) {
            let trimmed = String(line)
            if trimmed.isEmpty {
                flush()
                continue
            }
            if trimmed.hasPrefix("worktree ") {
                currentPath = String(trimmed.dropFirst("worktree ".count))
            } else if trimmed.hasPrefix("HEAD ") {
                currentHead = String(trimmed.dropFirst("HEAD ".count))
            } else if trimmed.hasPrefix("branch ") {
                let full = String(trimmed.dropFirst("branch ".count))
                currentBranch = full.hasPrefix("refs/heads/")
                    ? String(full.dropFirst("refs/heads/".count))
                    : full
            } else if trimmed == "bare" {
                isBare = true
            } else if trimmed == "detached" {
                isDetached = true
            } else if trimmed == "prunable" || trimmed.hasPrefix("prunable ") {
                isPrunable = true
            }
        }
        flush()
        return records
    }
}
