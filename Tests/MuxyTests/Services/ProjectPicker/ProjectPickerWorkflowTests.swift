import Foundation
import Testing

@testable import Muxy

@MainActor
@Suite("ProjectPickerWorkflow", .timeLimit(.minutes(1)))
struct ProjectPickerWorkflowTests {
    @Test("new input applies only latest directory snapshot")
    func latestDirectorySnapshotWins() async throws {
        let loader = ProjectPickerWorkflowTestDirectoryLoader()
        let workflow = ProjectPickerWorkflow(
            defaultDisplayPath: "~/",
            homeDirectory: "/Users/alice",
            projectPaths: [],
            directoryLoader: { await loader.load($0) },
            reloadDelay: .zero,
            loadingMessageDelay: .seconds(5)
        )

        _ = workflow.setInput("~/First")
        try #require(await loader.waitForRequest("~/First"))

        _ = workflow.setInput("~/Second")
        try #require(await loader.waitForRequest("~/Second"))

        await loader.resolve(
            input: "~/Second",
            snapshot: ProjectPickerDirectorySnapshot(rows: ["Second"], readFailed: false)
        )
        try #require(await waitUntil { workflow.session.rows.map(\.name) == ["Second"] })

        await loader.resolve(
            input: "~/First",
            snapshot: ProjectPickerDirectorySnapshot(rows: ["First"], readFailed: false)
        )
        try? await Task.sleep(for: .milliseconds(20))

        #expect(workflow.session.rows.map(\.name) == ["Second"])
    }

    @Test("loading message appears only while reload is active")
    func loadingMessagePolicy() async throws {
        let loader = ProjectPickerWorkflowTestDirectoryLoader()
        let slowWorkflow = ProjectPickerWorkflow(
            defaultDisplayPath: "~/Slow",
            homeDirectory: "/Users/alice",
            projectPaths: [],
            directoryLoader: { await loader.load($0) },
            reloadDelay: .zero,
            loadingMessageDelay: .milliseconds(10)
        )

        _ = slowWorkflow.setInput("~/Slow")
        try #require(await loader.waitForRequest("~/Slow"))
        try #require(await waitUntil { slowWorkflow.session.directoryLoadState.showsMessage })
        #expect(slowWorkflow.session.directoryLoadState == .loading(showsMessage: true))

        let fastWorkflow = ProjectPickerWorkflow(
            defaultDisplayPath: "~/Fast",
            homeDirectory: "/Users/alice",
            projectPaths: [],
            directoryLoader: { _ in ProjectPickerDirectorySnapshot(rows: ["Fast"], readFailed: false) },
            reloadDelay: .zero,
            loadingMessageDelay: .milliseconds(50)
        )

        _ = fastWorkflow.setInput("~/Fast")
        try #require(await waitUntil { fastWorkflow.session.directoryLoadState == .loaded })
        try? await Task.sleep(for: .milliseconds(80))

        #expect(fastWorkflow.session.directoryLoadState == .loaded)
    }

    @Test("folder search applies only the latest query and confirms the selected absolute path")
    func latestFolderSearchWins() async throws {
        let loader = ProjectPickerWorkflowTestFolderSearchLoader()
        let workflow = ProjectPickerWorkflow(
            defaultDisplayPath: "~/Projects/",
            homeDirectory: "/Users/alice",
            projectPaths: [],
            folderSearchPreparer: { _ in },
            folderSearchLoader: { query, _, _, _ in await loader.load(query) },
            reloadDelay: .zero,
            loadingMessageDelay: .seconds(5)
        )
        let secondResult = ProjectPickerFolderSearchResult(
            name: "muxy",
            path: "/Users/alice/Projects/muxy",
            displayPath: "~/Projects/muxy/"
        )

        _ = workflow.setInput("mu")
        try #require(await loader.waitForRequest("mu"))
        _ = workflow.setInput("muxy")
        try #require(await loader.waitForRequest("muxy"))

        await loader.resolve(
            query: "muxy",
            snapshot: ProjectPickerFolderSearchSnapshot(results: [secondResult], readFailed: false)
        )
        try #require(await waitUntil { workflow.session.searchResults == [secondResult] })

        await loader.resolve(
            query: "mu",
            snapshot: ProjectPickerFolderSearchSnapshot(
                results: [
                    ProjectPickerFolderSearchResult(
                        name: "music",
                        path: "/Users/alice/Music",
                        displayPath: "~/Music/"
                    ),
                ],
                readFailed: false
            )
        )
        try? await Task.sleep(for: .milliseconds(20))

        #expect(workflow.session.searchResults == [secondResult])
        #expect(workflow.handle(.openHighlighted) == [
            .confirmProjectPath(path: secondResult.path, createIfMissing: false),
        ])
        #expect(workflow.handle(.confirmTypedPath) == [
            .confirmProjectPath(path: secondResult.path, createIfMissing: false),
        ])
    }

    @Test("typed path confirmation emits external requests")
    func typedPathConfirmationRequests() throws {
        let existingPath = FileManager.default.temporaryDirectory
            .appendingPathComponent("muxy-project-picker-workflow-existing-\(UUID().uuidString)", isDirectory: true)
            .standardizedFileURL
        try FileManager.default.createDirectory(at: existingPath, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: existingPath) }

        let existingWorkflow = ProjectPickerWorkflow(defaultDisplayPath: existingPath.path, projectPaths: [])
        _ = existingWorkflow.setInput(existingPath.path)
        #expect(existingWorkflow.handle(.confirmTypedPath) == [
            .confirmProjectPath(path: existingPath.path, createIfMissing: false),
        ])

        let missingPath = FileManager.default.temporaryDirectory
            .appendingPathComponent("muxy-project-picker-workflow-\(UUID().uuidString)", isDirectory: true)
            .standardizedFileURL
            .path
        let workflow = ProjectPickerWorkflow(defaultDisplayPath: missingPath, projectPaths: [])
        _ = workflow.setInput(missingPath)

        #expect(workflow.handle(.confirmTypedPath) == [.askCreateDirectory(path: missingPath)])
        #expect(workflow.handleCreateDirectoryDecision(path: missingPath, accepted: false) == [])
        #expect(workflow.handleCreateDirectoryDecision(path: missingPath, accepted: true) == [
            .confirmProjectPath(path: missingPath, createIfMissing: true),
        ])
    }

    @Test("confirmation result requests dismissal or failure presentation")
    func confirmationResultHandling() {
        let workflow = ProjectPickerWorkflow(defaultDisplayPath: "~/", homeDirectory: "/Users/alice", projectPaths: [])

        #expect(workflow.handleProjectPathConfirmationResult(.success, path: "/tmp/muxy") == [.dismiss])
        #expect(workflow.handleProjectPathConfirmationResult(.notDirectory, path: "/tmp/muxy") == [
            .showFailure(ProjectPickerConfirmationFailurePresentation(result: .notDirectory, path: "/tmp/muxy")),
        ])
    }

    @Test("finder and settings actions emit edge requests")
    func edgeSideEffectRequests() {
        let workflow = ProjectPickerWorkflow(defaultDisplayPath: "~/", homeDirectory: "/Users/alice", projectPaths: [])

        #expect(workflow.chooseWithFinder() == [.dismiss, .chooseFinder])
        #expect(workflow.editDefaultLocation() == [.dismiss, .openSettingsFocusedOnDefaultLocation])
    }

    private func waitUntil(
        timeout: Duration = .seconds(5),
        condition: @escaping () async -> Bool
    ) async -> Bool {
        let start = ContinuousClock.now
        while ContinuousClock.now - start < timeout {
            if await condition() { return true }
            try? await Task.sleep(for: .milliseconds(5))
        }
        return false
    }
}

private actor ProjectPickerWorkflowTestDirectoryLoader {
    private var requests: Set<String> = []
    private var requestWaiters: [String: [UUID: CheckedContinuation<Bool, Never>]] = [:]
    private var continuations: [String: CheckedContinuation<ProjectPickerDirectorySnapshot, Never>] = [:]

    func load(_ pathState: ProjectPickerPathState) async -> ProjectPickerDirectorySnapshot {
        requests.insert(pathState.input)
        if let waiters = requestWaiters.removeValue(forKey: pathState.input) {
            for continuation in waiters.values {
                continuation.resume(returning: true)
            }
        }
        return await withCheckedContinuation { continuation in
            continuations[pathState.input] = continuation
        }
    }

    func waitForRequest(_ input: String, timeout: Duration = .seconds(5)) async -> Bool {
        guard !requests.contains(input) else { return true }
        let waiterID = UUID()
        return await withTaskCancellationHandler {
            await withCheckedContinuation { continuation in
                requestWaiters[input, default: [:]][waiterID] = continuation
                Task { [weak self] in
                    try? await Task.sleep(for: timeout)
                    await self?.resumeRequestWaiter(input: input, id: waiterID, result: false)
                }
            }
        } onCancel: {
            Task { await self.resumeRequestWaiter(input: input, id: waiterID, result: false) }
        }
    }

    private func resumeRequestWaiter(input: String, id: UUID, result: Bool) {
        guard let continuation = requestWaiters[input]?.removeValue(forKey: id) else { return }
        if requestWaiters[input]?.isEmpty == true {
            requestWaiters[input] = nil
        }
        continuation.resume(returning: result)
    }

    func resolve(input: String, snapshot: ProjectPickerDirectorySnapshot) {
        continuations.removeValue(forKey: input)?.resume(returning: snapshot)
    }
}

private actor ProjectPickerWorkflowTestFolderSearchLoader {
    private var requests: Set<String> = []
    private var requestWaiters: [String: [UUID: CheckedContinuation<Bool, Never>]] = [:]
    private var continuations: [String: CheckedContinuation<ProjectPickerFolderSearchSnapshot, Never>] = [:]

    func load(_ query: String) async -> ProjectPickerFolderSearchSnapshot {
        requests.insert(query)
        if let waiters = requestWaiters.removeValue(forKey: query) {
            for continuation in waiters.values {
                continuation.resume(returning: true)
            }
        }
        return await withCheckedContinuation { continuation in
            continuations[query] = continuation
        }
    }

    func waitForRequest(_ query: String, timeout: Duration = .seconds(5)) async -> Bool {
        guard !requests.contains(query) else { return true }
        let waiterID = UUID()
        return await withTaskCancellationHandler {
            await withCheckedContinuation { continuation in
                requestWaiters[query, default: [:]][waiterID] = continuation
                Task { [weak self] in
                    try? await Task.sleep(for: timeout)
                    await self?.resumeRequestWaiter(query: query, id: waiterID, result: false)
                }
            }
        } onCancel: {
            Task { await self.resumeRequestWaiter(query: query, id: waiterID, result: false) }
        }
    }

    private func resumeRequestWaiter(query: String, id: UUID, result: Bool) {
        guard let continuation = requestWaiters[query]?.removeValue(forKey: id) else { return }
        if requestWaiters[query]?.isEmpty == true {
            requestWaiters[query] = nil
        }
        continuation.resume(returning: result)
    }

    func resolve(query: String, snapshot: ProjectPickerFolderSearchSnapshot) {
        continuations.removeValue(forKey: query)?.resume(returning: snapshot)
    }
}
