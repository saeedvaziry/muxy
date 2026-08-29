import Darwin
import Foundation
import os

private let logger = Logger(subsystem: "app.muxy", category: "WorktreeStore")

enum WorktreeMutationError: LocalizedError {
    case projectRemovalInProgress
    case concurrentModification

    var errorDescription: String? {
        switch self {
        case .projectRemovalInProgress:
            "This project is being removed."
        case .concurrentModification:
            "Worktrees changed while they were being refreshed."
        }
    }
}

enum WorktreeCleanupResult: Equatable {
    case removed
    case retained
    case unknown
    case preservedMissingRepository
    case preservedUnverifiedDirectory

    var directoryRemoved: Bool? {
        switch self {
        case .removed:
            true
        case .retained,
             .preservedMissingRepository,
             .preservedUnverifiedDirectory:
            false
        case .unknown:
            nil
        }
    }
}

struct WorktreeCreationRequest {
    let name: String
    let path: String
    let branch: String
    let createBranch: Bool
    let baseBranch: String?
    let runSetup: Bool
    let projectHookApproval: WorktreeConfig.ProjectHookApproval?

    init(
        name: String,
        path: String,
        branch: String,
        createBranch: Bool,
        baseBranch: String?,
        runSetup: Bool = false,
        projectHookApproval: WorktreeConfig.ProjectHookApproval? = nil
    ) {
        self.name = name
        self.path = path
        self.branch = branch
        self.createBranch = createBranch
        self.baseBranch = baseBranch
        self.runSetup = runSetup
        self.projectHookApproval = projectHookApproval
    }
}

struct WorktreeRemovalRequest {
    let worktree: Worktree
    let projectID: UUID
    let repoPath: String
    let context: WorkspaceContext
    let projectHookApproval: WorktreeConfig.ProjectHookApproval?
}

@MainActor
@Observable
final class WorktreeStore {
    private static let maxRefreshAttempts = 3

    private typealias FileIdentity = WorktreeProcessQuiescer.DirectoryIdentity

    struct CleanupDependencies: Sendable {
        let worktreeRegistration: @Sendable (
            String,
            String,
            WorkspaceContext,
            TimeInterval
        ) async throws -> GitWorktreeRegistration
        let pathExists: @Sendable (String, WorkspaceContext, TimeInterval) async throws -> Bool

        static let live = CleanupDependencies(
            worktreeRegistration: { repoPath, path, context, timeout in
                try await GitWorktreeService.shared.worktreeRegistration(
                    repoPath: repoPath,
                    path: path,
                    context: context,
                    timeout: timeout
                )
            },
            pathExists: { path, context, timeout in
                try await context.fileOps.exists(at: path, timeout: timeout)
            }
        )
    }

    private enum CleanupPlan {
        case registered
        case staleManaged(projectID: UUID, identity: FileIdentity)
        case finished(WorktreeCleanupResult)
    }

    private enum StaleRemovalResult {
        case removedPath(String)
        case finished(WorktreeCleanupResult)
    }

    private struct CleanupOperation {
        let worktree: Worktree
        let projectID: UUID?
        let repoPath: String
        let context: WorkspaceContext
        let force: Bool
        let deadline: OperationDeadline
        let dependencies: CleanupDependencies
    }

    private(set) var worktrees: [UUID: [Worktree]] = [:]
    private(set) var preparingRemovalWorktreeIDs: Set<UUID> = []
    private(set) var removingWorktreeIDs: Set<UUID> = []
    private var projectIDsByPath: [String: Set<UUID>] = [:]
    private var localProjectIDs: Set<UUID> = []
    private var projectsBeingRemoved: Set<UUID> = []
    private var activeProjectMutationCounts: [UUID: Int] = [:]
    private var projectMutationWaiters: [UUID: [CheckedContinuation<Void, Never>]] = [:]
    var onWorktreesChanged: ((UUID, UUID?) -> Void)?
    private let persistence: any WorktreePersisting
    private let listGitWorktrees: @Sendable (String) async throws -> [GitWorktreeRecord]
    private let addGitWorktree: @Sendable (String, String, String, Bool, String?) async throws -> Void
    private let runWorktreeSetup: (String, Worktree, WorktreeConfig.ProjectHookApproval?) async -> Void
    private let pathResolver: any WorkspacePathResolving

    init(
        persistence: any WorktreePersisting,
        listGitWorktrees: @escaping @Sendable (String) async throws -> [GitWorktreeRecord] = {
            try await GitWorktreeService.shared.listWorktrees(repoPath: $0)
        },
        addGitWorktree: @escaping @Sendable (String, String, String, Bool, String?) async throws -> Void = {
            try await GitWorktreeService.shared.addWorktree(
                repoPath: $0,
                path: $1,
                branch: $2,
                createBranch: $3,
                baseBranch: $4
            )
        },
        runWorktreeSetup: @escaping (String, Worktree, WorktreeConfig.ProjectHookApproval?) async -> Void = {
            await WorktreeSetupRunner.run(sourceProjectPath: $0, worktree: $1, projectHookApproval: $2)
        },
        pathResolver: any WorkspacePathResolving = WorkspacePathResolver.live,
        projects: [Project] = []
    ) {
        self.persistence = persistence
        self.listGitWorktrees = listGitWorktrees
        self.addGitWorktree = addGitWorktree
        self.runWorktreeSetup = runWorktreeSetup
        self.pathResolver = pathResolver
        guard !projects.isEmpty else { return }
        loadAll(projects: projects)
    }

    func loadAll(projects: [Project]) {
        for project in projects {
            trackProject(project)
            guard !projectsBeingRemoved.contains(project.id) else { continue }
            do {
                var loaded = try persistence.loadWorktrees(projectID: project.id)
                let originalIDs = loaded.map(\.id)
                loaded = repairDuplicateIDs(loaded, projectID: project.id)
                if !loaded.contains(where: \.isPrimary) {
                    loaded.insert(makePrimary(for: project), at: 0)
                }
                if loaded.map(\.id) != originalIDs {
                    do {
                        try persistence.saveWorktrees(loaded, projectID: project.id)
                    } catch {
                        logger.error("Failed to persist repaired worktree identifiers for project \(project.id): \(error)")
                    }
                }
                setWorktrees(sortPrimaryFirst(loaded), for: project.id)
            } catch {
                logger.error("Failed to load worktrees for project \(project.id): \(error)")
                setWorktrees([makePrimary(for: project)], for: project.id)
                save(projectID: project.id)
            }
        }
    }

    func ensurePrimary(for project: Project) {
        trackProject(project)
        guard !projectsBeingRemoved.contains(project.id) else { return }
        var list = worktrees[project.id] ?? []
        if list.contains(where: \.isPrimary) {
            return
        }
        list.insert(makePrimary(for: project), at: 0)
        setWorktrees(sortPrimaryFirst(list), for: project.id)
        save(projectID: project.id)
        onWorktreesChanged?(project.id, list.first(where: \.isPrimary)?.id)
    }

    func list(for projectID: UUID) -> [Worktree] {
        worktrees[projectID] ?? []
    }

    func projectID(forWorktreePath path: String) -> UUID? {
        guard let projectIDs = projectIDsByPath[path] else { return nil }
        return projectIDs.first(where: { localProjectIDs.contains($0) }) ?? projectIDs.first
    }

    func primary(for projectID: UUID) -> Worktree? {
        list(for: projectID).first(where: { $0.isPrimary })
    }

    func worktree(projectID: UUID, worktreeID: UUID) -> Worktree? {
        list(for: projectID).first(where: { $0.id == worktreeID })
    }

    func preferred(for projectID: UUID, matching preferredID: UUID?) -> Worktree? {
        let list = list(for: projectID)
        return list.first(where: { $0.id == preferredID })
            ?? list.first(where: { $0.isPrimary })
            ?? list.first
    }

    func markActive(projectID: UUID, worktreeID: UUID) {
        guard var list = worktrees[projectID],
              let index = list.firstIndex(where: { $0.id == worktreeID })
        else { return }
        list[index].lastActiveAt = Date()
        setWorktrees(list, for: projectID)
        save(projectID: projectID)
        onWorktreesChanged?(projectID, worktreeID)
    }

    func add(_ worktree: Worktree, to projectID: UUID, context: WorkspaceContext = .local) {
        guard !projectsBeingRemoved.contains(projectID) else { return }
        store(worktree, for: projectID, context: context)
    }

    @discardableResult
    private func store(
        _ worktree: Worktree,
        for projectID: UUID,
        context: WorkspaceContext,
        identityPath: String? = nil,
        identityPathsByWorktreeID: [UUID: String] = [:]
    ) -> Worktree {
        var list = worktrees[projectID] ?? []
        let key = GitWorktreeService.canonicalPath(identityPath ?? worktree.path, context: context)
        let storedWorktree: Worktree
        if let index = list.firstIndex(where: {
            let existingPath = identityPathsByWorktreeID[$0.id] ?? $0.path
            return !$0.isPrimary && GitWorktreeService.canonicalPath(existingPath, context: context) == key
        }) {
            let existing = list[index]
            storedWorktree = Worktree(
                id: existing.id,
                name: worktree.name,
                path: worktree.path,
                branch: worktree.branch,
                source: worktree.source,
                isPrimary: worktree.isPrimary,
                createdAt: existing.createdAt,
                lastActiveAt: existing.lastActiveAt
            )
            list[index] = storedWorktree
        } else {
            storedWorktree = worktree
            list.append(storedWorktree)
        }
        setWorktrees(sortPrimaryFirst(list), for: projectID)
        save(projectID: projectID)
        onWorktreesChanged?(projectID, storedWorktree.id)
        return storedWorktree
    }

    func createWorktree(
        project: Project,
        request: WorktreeCreationRequest,
        context: WorkspaceContext = .local
    ) async throws -> Worktree {
        guard beginProjectMutation(project.id) else {
            throw WorktreeMutationError.projectRemovalInProgress
        }
        var requiresRefresh = false
        defer {
            endProjectMutation(project.id)
            if requiresRefresh {
                scheduleRefresh(project: project, context: context)
            }
        }
        let parentPath = parentDirectory(of: request.path, context: context)
        try await context.fileOps.makeDirectory(at: parentPath)

        try await addWorktreeForContext(project: project, request: request, context: context)
        let worktree = Worktree(
            name: request.name,
            path: request.path,
            branch: request.branch,
            isPrimary: false
        )
        var storedWorktree: Worktree?
        for _ in 0 ..< Self.maxRefreshAttempts {
            let snapshot = worktrees[project.id] ?? []
            do {
                let identityPaths = try await pathResolver.resolve(
                    paths: [request.path] + snapshot.map(\.path),
                    relativeTo: project.path,
                    context: context,
                    timeout: SSHCommandRunner.defaultTimeout
                )
                guard identityPaths.count == snapshot.count + 1 else {
                    throw WorkspacePathResolverError.invalidOutput
                }
                guard Self.hasSameRefreshState(snapshot, worktrees[project.id] ?? []) else { continue }
                var identityPathsByWorktreeID: [UUID: String] = [:]
                for (existingWorktree, resolution) in zip(snapshot, identityPaths.dropFirst()) {
                    identityPathsByWorktreeID[existingWorktree.id] = resolution.path
                }
                storedWorktree = store(
                    worktree,
                    for: project.id,
                    context: context,
                    identityPath: identityPaths.first?.path,
                    identityPathsByWorktreeID: identityPathsByWorktreeID
                )
                break
            } catch {
                logger.error("Failed to resolve paths after creating worktree for project \(project.id): \(error)")
                break
            }
        }
        if storedWorktree == nil {
            requiresRefresh = true
            storedWorktree = store(worktree, for: project.id, context: context)
        }
        let createdWorktree = storedWorktree ?? worktree
        if request.runSetup, !context.isRemote {
            await runWorktreeSetup(project.path, createdWorktree, request.projectHookApproval)
        }
        return createdWorktree
    }

    private func addWorktreeForContext(
        project: Project,
        request: WorktreeCreationRequest,
        context: WorkspaceContext
    ) async throws {
        guard context.isRemote else {
            try await addGitWorktree(
                project.path,
                request.path,
                request.branch,
                request.createBranch,
                request.baseBranch
            )
            return
        }
        try await GitWorktreeService.shared.addWorktree(
            repoPath: project.path,
            path: request.path,
            branch: request.branch,
            createBranch: request.createBranch,
            baseBranch: request.baseBranch,
            context: context
        )
    }

    private func parentDirectory(of path: String, context: WorkspaceContext) -> String {
        guard context.isRemote else {
            return URL(fileURLWithPath: path).deletingLastPathComponent().path
        }
        guard let slashIndex = path.lastIndex(of: "/") else { return "." }
        let parent = String(path[..<slashIndex])
        return parent.isEmpty ? "/" : parent
    }

    func remove(worktreeID: UUID, from projectID: UUID) {
        guard !projectsBeingRemoved.contains(projectID) else { return }
        removeWorktree(worktreeID, from: projectID)
    }

    private func removeWorktree(_ worktreeID: UUID, from projectID: UUID) {
        guard var list = worktrees[projectID] else { return }
        list.removeAll { $0.id == worktreeID && $0.canBeRemoved }
        setWorktrees(list, for: projectID)
        save(projectID: projectID)
        onWorktreesChanged?(projectID, worktreeID)
    }

    func isRemoving(worktreeID: UUID) -> Bool {
        removingWorktreeIDs.contains(worktreeID)
    }

    var hasRemovalPreparation: Bool {
        !preparingRemovalWorktreeIDs.isEmpty
    }

    func isPreparingRemoval(worktreeID: UUID) -> Bool {
        preparingRemovalWorktreeIDs.contains(worktreeID)
    }

    func isRemovalInProgress(worktreeID: UUID) -> Bool {
        isPreparingRemoval(worktreeID: worktreeID) || isRemoving(worktreeID: worktreeID)
    }

    func beginRemovalPreparation(worktree: Worktree, projectID: UUID) -> Bool {
        guard !projectsBeingRemoved.contains(projectID) else { return false }
        guard worktree.canBeRemoved, !isRemoving(worktreeID: worktree.id) else { return false }
        return preparingRemovalWorktreeIDs.insert(worktree.id).inserted
    }

    func endRemovalPreparation(worktreeID: UUID) {
        preparingRemovalWorktreeIDs.remove(worktreeID)
    }

    func beginRemoval(_ request: WorktreeRemovalRequest, onSuccess: @escaping @MainActor () -> Void) {
        let worktree = request.worktree
        let projectID = request.projectID
        guard worktree.canBeRemoved,
              !isPreparingRemoval(worktreeID: worktree.id),
              removingWorktreeIDs.insert(worktree.id).inserted
        else { return }
        guard beginProjectMutation(projectID) else {
            removingWorktreeIDs.remove(worktree.id)
            return
        }
        Task { [weak self] in
            defer { self?.endProjectMutation(projectID) }
            do {
                let cleanupResult = try await WorktreeStore.cleanupOnDisk(
                    worktree: worktree,
                    projectID: projectID,
                    repoPath: request.repoPath,
                    context: request.context,
                    projectHookApproval: request.projectHookApproval
                )
                self?.removingWorktreeIDs.remove(worktree.id)
                self?.removeWorktree(worktree.id, from: projectID)
                onSuccess()
                if cleanupResult == .preservedMissingRepository {
                    ToastState.shared.show(
                        title: L10n.string("Worktree removed from Muxy"),
                        body: L10n.string("The main repository is missing, so files were preserved at \"\(worktree.path)\".")
                    )
                } else if cleanupResult == .preservedUnverifiedDirectory {
                    ToastState.shared.show(
                        title: L10n.string("Worktree removed from Muxy"),
                        body: L10n.string("Git ownership could not be verified, so files were preserved at \"\(worktree.path)\".")
                    )
                }
            } catch {
                self?.removingWorktreeIDs.remove(worktree.id)
                ToastState.shared.show(
                    title: L10n.string("Could not remove worktree \"\(worktree.name)\""),
                    body: error.localizedDescription
                )
            }
        }
    }

    func refreshFromGit(project: Project, context: WorkspaceContext = .local) async throws -> [Worktree] {
        guard beginProjectMutation(project.id) else {
            throw WorktreeMutationError.projectRemovalInProgress
        }
        defer { endProjectMutation(project.id) }
        ensurePrimary(for: project)
        for _ in 0 ..< Self.maxRefreshAttempts {
            try Task.checkCancellation()
            let snapshot = worktrees[project.id] ?? []
            let records = try await listWorktreesForContext(project: project, context: context)
                .filter { !$0.isBare && !$0.isPrunable }
            var list = snapshot
            let paths = [project.path] + list.map(\.path) + records.map(\.path)
            let resolutions = try await pathResolver.resolve(
                paths: paths,
                relativeTo: project.path,
                context: context,
                timeout: SSHCommandRunner.defaultTimeout
            )
            guard resolutions.count == paths.count else {
                throw GitWorktreeService.GitWorktreeError.commandFailed("Failed to resolve worktree paths.")
            }
            let current = worktrees[project.id] ?? []
            guard Self.hasSameRefreshState(snapshot, current) else { continue }
            var currentByID: [UUID: Worktree] = [:]
            for worktree in current {
                currentByID[worktree.id] = worktree
            }
            for index in list.indices {
                list[index].lastActiveAt = currentByID[list[index].id]?.lastActiveAt
            }
            let projectKey = GitWorktreeService.canonicalPath(resolutions[0].path, context: context)
            let listResolutions = resolutions.dropFirst().prefix(list.count)
            let recordResolutions = resolutions.dropFirst(1 + list.count)
            var pathKeysByID: [UUID: String] = [:]
            for (worktree, resolution) in zip(list, listResolutions) {
                pathKeysByID[worktree.id] = GitWorktreeService.canonicalPath(resolution.path, context: context)
            }
            let resolvedRecordKeys = recordResolutions.map {
                GitWorktreeService.canonicalPath($0.path, context: context)
            }
            let recordKeys = Set(resolvedRecordKeys)

            if let primaryIndex = list.firstIndex(where: \.isPrimary) {
                list[primaryIndex].path = project.path
                list[primaryIndex].name = project.name
                pathKeysByID[list[primaryIndex].id] = projectKey
            } else {
                let primary = makePrimary(for: project)
                list.insert(primary, at: 0)
                pathKeysByID[primary.id] = projectKey
            }

            var existingByKey: [String: Worktree] = [:]
            for worktree in list {
                let key = pathKeysByID[worktree.id]
                    ?? GitWorktreeService.canonicalPath(worktree.path, context: context)
                guard let existing = existingByKey[key] else {
                    existingByKey[key] = worktree
                    continue
                }
                if worktree.isPrimary && !existing.isPrimary
                    || existing.isExternallyManaged && !worktree.isExternallyManaged
                {
                    existingByKey[key] = worktree
                }
            }

            for (record, recordKey) in zip(records, resolvedRecordKeys) {
                if recordKey == projectKey {
                    if let primaryIndex = list.firstIndex(where: \.isPrimary) {
                        list[primaryIndex].branch = record.branch
                    }
                    continue
                }

                if let existing = existingByKey[recordKey],
                   let index = list.firstIndex(where: { $0.id == existing.id })
                {
                    if list[index].isPrimary {
                        list[index].name = project.name
                        list[index].path = project.path
                    } else if record.branch != nil, list[index].name == list[index].branch {
                        list[index].name = defaultName(for: record)
                    }
                    list[index].branch = record.branch
                    continue
                }

                let imported = Worktree(
                    name: defaultName(for: record),
                    path: record.path,
                    branch: record.branch,
                    source: .external,
                    isPrimary: false
                )
                list.append(imported)
                pathKeysByID[imported.id] = recordKey
            }

            let filtered = list.filter {
                let key = pathKeysByID[$0.id]
                    ?? GitWorktreeService.canonicalPath($0.path, context: context)
                if !$0.isPrimary, key == projectKey {
                    return false
                }
                return !$0.isExternallyManaged || recordKeys.contains(key)
            }
            let sorted = sortPrimaryFirst(collapseDuplicatePaths(
                filtered,
                context: context,
                pathKeysByID: pathKeysByID
            ))
            try Task.checkCancellation()
            setWorktrees(sorted, for: project.id)
            save(projectID: project.id)
            onWorktreesChanged?(project.id, nil)
            return sorted
        }
        throw WorktreeMutationError.concurrentModification
    }

    private static func hasSameRefreshState(_ lhs: [Worktree], _ rhs: [Worktree]) -> Bool {
        guard lhs.count == rhs.count else { return false }
        return zip(lhs, rhs).allSatisfy {
            $0.id == $1.id
                && $0.name == $1.name
                && $0.path == $1.path
                && $0.branch == $1.branch
                && $0.source == $1.source
                && $0.isPrimary == $1.isPrimary
                && $0.createdAt == $1.createdAt
        }
    }

    private func scheduleRefresh(project: Project, context: WorkspaceContext) {
        Task { [weak self] in
            guard let self else { return }
            do {
                _ = try await refreshFromGit(project: project, context: context)
            } catch {
                logger.error("Worktree reconciliation failed for project \(project.id): \(error)")
            }
        }
    }

    private func repairDuplicateIDs(_ list: [Worktree], projectID: UUID) -> [Worktree] {
        var seen: Set<UUID> = []
        return list.map { worktree in
            guard seen.insert(worktree.id).inserted else {
                logger.warning("Repairing duplicate worktree identifier \(worktree.id) for project \(projectID)")
                return Worktree(
                    name: worktree.name,
                    path: worktree.path,
                    branch: worktree.branch,
                    source: worktree.source,
                    isPrimary: worktree.isPrimary,
                    createdAt: worktree.createdAt,
                    lastActiveAt: worktree.lastActiveAt
                )
            }
            return worktree
        }
    }

    private func collapseDuplicatePaths(
        _ list: [Worktree],
        context: WorkspaceContext,
        pathKeysByID: [UUID: String]
    ) -> [Worktree] {
        var indexByKey: [String: Int] = [:]
        var result: [Worktree] = []
        for worktree in list {
            guard !worktree.isPrimary else {
                result.append(worktree)
                continue
            }
            let key = pathKeysByID[worktree.id]
                ?? GitWorktreeService.canonicalPath(worktree.path, context: context)
            guard let existingIndex = indexByKey[key] else {
                indexByKey[key] = result.count
                result.append(worktree)
                continue
            }
            if result[existingIndex].isExternallyManaged, !worktree.isExternallyManaged {
                result[existingIndex] = worktree
            }
        }
        return result
    }

    private func listWorktreesForContext(
        project: Project,
        context: WorkspaceContext
    ) async throws -> [GitWorktreeRecord] {
        guard context.isRemote else {
            return try await listGitWorktrees(project.path)
        }
        return try await GitWorktreeService.shared.listWorktrees(repoPath: project.path, context: context)
    }

    @discardableResult
    static func cleanupOnDisk(
        worktree: Worktree,
        projectID: UUID? = nil,
        repoPath: String,
        context: WorkspaceContext = .local,
        projectHookApproval: WorktreeConfig.ProjectHookApproval? = nil,
        teardownGlobalConfigURL: URL = WorktreeConfig.globalConfigURL(),
        force: Bool = true,
        timeout: TimeInterval = GitWorktreeService.defaultWorktreeRemovalTimeout,
        dependencies: CleanupDependencies = .live,
        teardownEmit: @Sendable @escaping (WorktreeTeardownOutputLine) -> Void = { _ in }
    ) async throws -> WorktreeCleanupResult {
        guard worktree.canBeRemoved else {
            return await context.fileOps.exists(at: worktree.path) ? .retained : .removed
        }
        guard context.isRemote || FileManager.default.fileExists(atPath: repoPath) else {
            return .preservedMissingRepository
        }
        let deadline = OperationDeadline(timeout: timeout)
        let operation = CleanupOperation(
            worktree: worktree,
            projectID: projectID,
            repoPath: repoPath,
            context: context,
            force: force,
            deadline: deadline,
            dependencies: dependencies
        )
        let plan = try await cleanupPlan(for: operation)
        if case let .finished(result) = plan {
            return result
        }
        if !context.isRemote {
            try await WorktreeTeardownRunner.run(
                sourceProjectPath: repoPath,
                worktree: worktree,
                projectHookApproval: projectHookApproval,
                timeout: deadline.remaining(),
                emit: teardownEmit,
                globalConfigURL: teardownGlobalConfigURL
            )
        }
        let removedPath: String
        switch plan {
        case .registered:
            removedPath = try await GitWorktreeService.shared.removeWorktree(
                repoPath: repoPath,
                path: worktree.path,
                force: force,
                context: context,
                timeout: deadline.remaining()
            )
        case let .staleManaged(projectID, identity):
            switch try await removeStaleManagedCheckout(
                operation: operation,
                projectID: projectID,
                identity: identity
            ) {
            case let .removedPath(path):
                removedPath = path
            case let .finished(result):
                return result
            }
        case let .finished(result):
            return result
        }

        let directoryRemoved = await (try? context.fileOps.exists(
            at: removedPath,
            timeout: deadline.remaining()
        )).map { !$0 }
        guard directoryRemoved == true, !context.isRemote, !worktree.isExternallyManaged else {
            return cleanupResult(directoryRemoved: directoryRemoved)
        }
        await removeParentDirectoryIfEmpty(for: removedPath)
        return .removed
    }

    private static func cleanupPlan(for operation: CleanupOperation) async throws -> CleanupPlan {
        let worktree = operation.worktree
        if !operation.context.isRemote, !isUnambiguousLocalPath(worktree.path) {
            throw GitWorktreeService.GitWorktreeError.notRegistered
        }
        let registration = try await operation.dependencies.worktreeRegistration(
            operation.repoPath,
            worktree.path,
            operation.context,
            operation.deadline.remaining()
        )
        guard !registration.isRegistered else { return .registered }
        guard try await operation.dependencies.pathExists(
            registration.path,
            operation.context,
            operation.deadline.remaining()
        )
        else {
            if !operation.context.isRemote {
                await removeParentDirectoryIfEmpty(for: worktree.path)
            }
            return .finished(.removed)
        }
        guard !operation.context.isRemote else {
            throw GitWorktreeService.GitWorktreeError.notRegistered
        }
        guard operation.force,
              let projectID = operation.projectID,
              isMuxyManagedCheckout(worktree, projectID: projectID)
        else {
            throw GitWorktreeService.GitWorktreeError.notRegistered
        }
        guard checkoutReferencesRepository(worktreePath: worktree.path, repoPath: operation.repoPath),
              let identity = fileIdentity(at: worktree.path)
        else {
            return .finished(.preservedUnverifiedDirectory)
        }
        return .staleManaged(projectID: projectID, identity: identity)
    }

    private static func removeStaleManagedCheckout(
        operation: CleanupOperation,
        projectID: UUID,
        identity: FileIdentity
    ) async throws -> StaleRemovalResult {
        let worktree = operation.worktree
        let isNowRegistered = try await operation.dependencies.worktreeRegistration(
            operation.repoPath,
            worktree.path,
            operation.context,
            operation.deadline.remaining()
        ).isRegistered
        guard !isNowRegistered else {
            throw GitWorktreeService.GitWorktreeError.commandFailed("Worktree changed during removal.")
        }
        guard try await operation.dependencies.pathExists(
            worktree.path,
            operation.context,
            operation.deadline.remaining()
        )
        else {
            await removeParentDirectoryIfEmpty(for: worktree.path)
            return .finished(.removed)
        }
        guard isMuxyManagedCheckout(worktree, projectID: projectID),
              checkoutReferencesRepository(worktreePath: worktree.path, repoPath: operation.repoPath),
              fileIdentity(at: worktree.path) == identity
        else {
            return .finished(.preservedUnverifiedDirectory)
        }
        try await WorktreeProcessQuiescer.quiesce(
            path: worktree.path,
            matching: identity,
            timeout: operation.deadline.remaining()
        )
        guard isMuxyManagedCheckout(worktree, projectID: projectID),
              checkoutReferencesRepository(worktreePath: worktree.path, repoPath: operation.repoPath),
              fileIdentity(at: worktree.path) == identity
        else {
            return .finished(.preservedUnverifiedDirectory)
        }
        let isRegisteredAfterQuiescing = try await operation.dependencies.worktreeRegistration(
            operation.repoPath,
            worktree.path,
            operation.context,
            operation.deadline.remaining()
        ).isRegistered
        guard !isRegisteredAfterQuiescing else {
            throw GitWorktreeService.GitWorktreeError.commandFailed("Worktree changed during removal.")
        }
        let quarantine = try await quarantineManagedCheckout(
            worktree.path,
            projectID: projectID,
            matching: identity
        )
        guard fileIdentity(at: quarantine.path) == identity else {
            try await restoreOrReport(quarantine, to: worktree.path)
            return .finished(.preservedUnverifiedDirectory)
        }
        do {
            try await operation.context.fileOps.removeItem(
                at: quarantine.path,
                timeout: operation.deadline.remaining()
            )
        } catch {
            if await operation.context.fileOps.exists(at: quarantine.path) {
                try await restoreOrReport(quarantine, to: worktree.path)
                throw error
            }
        }
        return .removedPath(worktree.path)
    }

    private static func cleanupResult(directoryRemoved: Bool?) -> WorktreeCleanupResult {
        guard let directoryRemoved else { return .unknown }
        return directoryRemoved ? .removed : .retained
    }

    private static func isMuxyManagedCheckout(_ worktree: Worktree, projectID: UUID) -> Bool {
        guard worktree.source == .muxy, !worktree.isPrimary else { return false }
        let expectedRoot = MuxyFileStorage.worktreeRoot(forProjectID: projectID, create: false)
            .standardizedFileURL
        let target = URL(fileURLWithPath: worktree.path, isDirectory: true).standardizedFileURL
        guard target.deletingLastPathComponent() == expectedRoot,
              (try? FileManager.default.destinationOfSymbolicLink(atPath: expectedRoot.path)) == nil,
              (try? FileManager.default.destinationOfSymbolicLink(atPath: target.path)) == nil
        else { return false }
        let canonicalRoot = expectedRoot
            .resolvingSymlinksInPath()
            .standardizedFileURL
        let canonicalTarget = target
            .resolvingSymlinksInPath()
            .standardizedFileURL
        let checkoutsRoot = expectedRoot.deletingLastPathComponent()
        let canonicalCheckoutsRoot = checkoutsRoot.resolvingSymlinksInPath().standardizedFileURL
        return canonicalRoot.deletingLastPathComponent() == canonicalCheckoutsRoot
            && canonicalTarget.deletingLastPathComponent() == canonicalRoot
            && (try? FileManager.default.destinationOfSymbolicLink(atPath: checkoutsRoot.path)) == nil
    }

    private static func isUnambiguousLocalPath(_ path: String) -> Bool {
        guard path.hasPrefix("/") else { return false }
        let components = NSString(string: path).pathComponents
        guard !components.contains("."), !components.contains("..") else { return false }
        return (try? FileManager.default.destinationOfSymbolicLink(atPath: path)) == nil
    }

    nonisolated private static func fileIdentity(at path: String) -> FileIdentity? {
        WorktreeProcessQuiescer.directoryIdentity(at: path)
    }

    private static func quarantineManagedCheckout(
        _ path: String,
        projectID: UUID,
        matching identity: FileIdentity
    ) async throws -> URL {
        let quarantine = MuxyFileStorage.worktreeRoot(forProjectID: projectID, create: false)
            .appendingPathComponent(".muxy-removing-\(UUID().uuidString)", isDirectory: true)
        guard fileIdentity(at: path) == identity else {
            throw GitWorktreeService.GitWorktreeError.commandFailed("Worktree changed during removal.")
        }
        try await GitProcessRunner.offMainThrowing {
            guard fileIdentity(at: path) == identity else {
                throw GitWorktreeService.GitWorktreeError.commandFailed("Worktree changed during removal.")
            }
            try FileManager.default.moveItem(atPath: path, toPath: quarantine.path)
        }
        return quarantine
    }

    private static func restoreQuarantinedCheckout(_ quarantine: URL, to path: String) async throws {
        try await GitProcessRunner.offMainThrowing {
            guard !FileManager.default.fileExists(atPath: path) else {
                throw GitWorktreeService.GitWorktreeError.commandFailed("Worktree changed during removal.")
            }
            try FileManager.default.moveItem(atPath: quarantine.path, toPath: path)
        }
    }

    private static func restoreOrReport(_ quarantine: URL, to path: String) async throws {
        do {
            try await restoreQuarantinedCheckout(quarantine, to: path)
        } catch {
            logger.error(
                "Could not restore worktree to \(path, privacy: .public); files remain at \(quarantine.path, privacy: .public)"
            )
            throw GitWorktreeService.GitWorktreeError.commandFailed(
                "Worktree changed during removal. Files remain at \"\(quarantine.path)\"."
            )
        }
    }

    private static func checkoutReferencesRepository(worktreePath: String, repoPath: String) -> Bool {
        guard let checkoutGitDirectory = gitDirectoryReference(checkoutPath: worktreePath),
              let commonGitDirectory = commonGitDirectory(repoPath: repoPath)
        else { return false }
        return checkoutGitDirectory.deletingLastPathComponent()
            == commonGitDirectory.appendingPathComponent("worktrees", isDirectory: true)
    }

    private static func commonGitDirectory(repoPath: String) -> URL? {
        let dotGit = URL(fileURLWithPath: repoPath, isDirectory: true).appendingPathComponent(".git")
        guard (try? FileManager.default.destinationOfSymbolicLink(atPath: dotGit.path)) == nil else { return nil }
        if (try? dotGit.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true {
            return dotGit.resolvingSymlinksInPath().standardizedFileURL
        }
        guard let adminDirectory = gitDirectoryReference(checkoutPath: repoPath),
              adminDirectory.deletingLastPathComponent().lastPathComponent == "worktrees"
        else { return nil }
        return adminDirectory.deletingLastPathComponent().deletingLastPathComponent()
    }

    private static func gitDirectoryReference(checkoutPath: String) -> URL? {
        let checkout = URL(fileURLWithPath: checkoutPath, isDirectory: true).standardizedFileURL
        let dotGit = checkout.appendingPathComponent(".git")
        guard (try? FileManager.default.destinationOfSymbolicLink(atPath: dotGit.path)) == nil,
              let attributes = try? FileManager.default.attributesOfItem(atPath: dotGit.path),
              attributes[.type] as? FileAttributeType == .typeRegular,
              let size = attributes[.size] as? NSNumber,
              size.intValue <= 4096,
              let contents = try? String(contentsOf: dotGit, encoding: .utf8),
              let firstLine = contents.split(whereSeparator: \Character.isNewline).first,
              firstLine.hasPrefix("gitdir: ")
        else { return nil }
        let reference = firstLine.dropFirst("gitdir: ".count).trimmingCharacters(in: .whitespaces)
        guard !reference.isEmpty else { return nil }
        return URL(fileURLWithPath: reference, relativeTo: checkout)
            .standardizedFileURL
            .resolvingSymlinksInPath()
    }

    nonisolated private static func removeParentDirectoryIfEmpty(for path: String) async {
        await GitProcessRunner.offMain {
            let parent = URL(fileURLWithPath: path).deletingLastPathComponent()
            _ = parent.path.withCString { Darwin.rmdir($0) }
        }
    }

    static func cleanupOnDisk(
        for project: Project,
        knownWorktrees: [Worktree],
        context: WorkspaceContext = .local
    ) async throws {
        let secondaryWorktrees = knownWorktrees.filter { $0.canBeRemoved && !$0.isExternallyManaged }
        for worktree in secondaryWorktrees {
            try await cleanupOnDisk(
                worktree: worktree,
                projectID: project.id,
                repoPath: project.path,
                context: context
            )
        }

        guard !context.isRemote, FileManager.default.fileExists(atPath: project.path) else { return }
        let root = MuxyFileStorage.worktreeRoot(forProjectID: project.id)
        guard FileManager.default.fileExists(atPath: root.path) else { return }
        let children = (try? FileManager.default.contentsOfDirectory(atPath: root.path)) ?? []
        guard children.isEmpty else { return }
        _ = root.path.withCString { Darwin.rmdir($0) }
    }

    func rename(worktreeID: UUID, in projectID: UUID, to newName: String) {
        guard !projectsBeingRemoved.contains(projectID) else { return }
        guard var list = worktrees[projectID],
              let index = list.firstIndex(where: { $0.id == worktreeID })
        else { return }
        list[index].name = newName
        setWorktrees(list, for: projectID)
        save(projectID: projectID)
        onWorktreesChanged?(projectID, worktreeID)
    }

    func updateBranch(worktreeID: UUID, in projectID: UUID, branch: String?) {
        guard !projectsBeingRemoved.contains(projectID) else { return }
        guard var list = worktrees[projectID],
              let index = list.firstIndex(where: { $0.id == worktreeID })
        else { return }
        list[index].branch = branch
        setWorktrees(list, for: projectID)
        save(projectID: projectID)
        onWorktreesChanged?(projectID, worktreeID)
    }

    func removeProject(_ projectID: UUID) {
        guard !projectsBeingRemoved.contains(projectID) else { return }
        removeProjectState(projectID)
    }

    func completeProjectRemoval(_ projectID: UUID) {
        removeProjectState(projectID)
    }

    private func removeProjectState(_ projectID: UUID) {
        if let existing = worktrees[projectID] {
            for worktree in existing {
                removePathOwnership(worktree.path, projectID: projectID)
            }
        }
        let removed = worktrees.removeValue(forKey: projectID)
        localProjectIDs.remove(projectID)
        projectsBeingRemoved.remove(projectID)
        pruneRemovalState()
        do {
            try persistence.removeWorktrees(projectID: projectID)
        } catch {
            logger.error("Failed to remove worktrees file for project \(projectID): \(error)")
        }
        if removed != nil {
            onWorktreesChanged?(projectID, nil)
        }
    }

    func beginProjectRemoval(_ projectID: UUID) async -> Bool {
        guard projectsBeingRemoved.insert(projectID).inserted else { return false }
        guard activeProjectMutationCounts[projectID, default: 0] > 0 else { return true }
        await withCheckedContinuation { continuation in
            projectMutationWaiters[projectID, default: []].append(continuation)
        }
        return true
    }

    func cancelProjectRemoval(_ projectID: UUID) {
        projectsBeingRemoved.remove(projectID)
    }

    func isProjectRemovalInProgress(_ projectID: UUID) -> Bool {
        projectsBeingRemoved.contains(projectID)
    }

    func restoreProjectWorktrees(_ list: [Worktree], for project: Project) {
        trackProject(project)
        guard !list.isEmpty else {
            removeProject(project.id)
            return
        }
        setWorktrees(list, for: project.id)
        save(projectID: project.id)
        onWorktreesChanged?(project.id, nil)
    }

    private func setWorktrees(_ list: [Worktree], for projectID: UUID) {
        if let previous = worktrees[projectID] {
            for worktree in previous {
                removePathOwnership(worktree.path, projectID: projectID)
            }
        }
        for worktree in list {
            projectIDsByPath[worktree.path, default: []].insert(projectID)
        }
        worktrees[projectID] = list
        pruneRemovalState()
    }

    private func pruneRemovalState() {
        let liveIDs = Set(worktrees.values.flatMap(\.self).map(\.id))
        preparingRemovalWorktreeIDs.formIntersection(liveIDs)
        removingWorktreeIDs.formIntersection(liveIDs)
    }

    private func trackProject(_ project: Project) {
        if project.isRemote {
            localProjectIDs.remove(project.id)
        } else {
            localProjectIDs.insert(project.id)
        }
    }

    private func removePathOwnership(_ path: String, projectID: UUID) {
        guard var projectIDs = projectIDsByPath[path] else { return }
        projectIDs.remove(projectID)
        if projectIDs.isEmpty {
            projectIDsByPath.removeValue(forKey: path)
        } else {
            projectIDsByPath[path] = projectIDs
        }
    }

    private func makePrimary(for project: Project) -> Worktree {
        Worktree(
            name: project.name,
            path: project.path,
            branch: nil,
            source: .muxy,
            isPrimary: true
        )
    }

    private func sortPrimaryFirst(_ list: [Worktree]) -> [Worktree] {
        let primary = list.filter(\.isPrimary)
        let others = list.filter { !$0.isPrimary }.sorted { $0.createdAt < $1.createdAt }
        return primary + others
    }

    private func save(projectID: UUID) {
        guard let list = worktrees[projectID] else { return }
        do {
            try persistence.saveWorktrees(list, projectID: projectID)
        } catch {
            logger.error("Failed to save worktrees for project \(projectID): \(error)")
        }
    }

    private func beginProjectMutation(_ projectID: UUID) -> Bool {
        guard !projectsBeingRemoved.contains(projectID) else { return false }
        activeProjectMutationCounts[projectID, default: 0] += 1
        return true
    }

    private func endProjectMutation(_ projectID: UUID) {
        guard let count = activeProjectMutationCounts[projectID] else { return }
        guard count == 1 else {
            activeProjectMutationCounts[projectID] = count - 1
            return
        }
        activeProjectMutationCounts.removeValue(forKey: projectID)
        let waiters = projectMutationWaiters.removeValue(forKey: projectID) ?? []
        for waiter in waiters {
            waiter.resume()
        }
    }

    private func defaultName(for record: GitWorktreeRecord) -> String {
        if let branch = record.branch?.trimmingCharacters(in: .whitespacesAndNewlines),
           !branch.isEmpty
        {
            return branch
        }
        return URL(fileURLWithPath: record.path).lastPathComponent
    }
}
