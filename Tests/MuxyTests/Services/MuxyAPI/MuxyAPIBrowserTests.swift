import Foundation
import Testing
import WebKit

@testable import Muxy

@Suite("MuxyAPI.Browser", .serialized)
@MainActor
struct MuxyAPIBrowserTests {
    private let testPath = "/tmp/test"

    private func makeAppState(projectID: UUID = UUID(), worktreeID: UUID = UUID()) -> AppState {
        UserDefaults.standard.removeObject(forKey: BrowserPreferences.enabledKey)
        let appState = AppState(
            selectionStore: SelectionStoreStub(),
            terminalViews: TerminalViewRemovingStub(),
            workspacePersistence: WorkspacePersistenceStub()
        )
        let key = WorktreeKey(projectID: projectID, worktreeID: worktreeID)
        let area = TabArea(projectPath: testPath)
        appState.activeProjectID = projectID
        appState.activeWorktreeID[projectID] = worktreeID
        appState.workspaceRoots[key] = .tabArea(area)
        appState.focusedAreaID[key] = area.id
        return appState
    }

    @Test("open returns a browser tab id")
    func openReturnsID() throws {
        let appState = makeAppState()
        let result = MuxyAPI.Browser.open(url: "https://example.com", appState: appState)
        let id = try result.get()
        let listed = MuxyAPI.Browser.list(appState: appState, profileStore: nil)
        #expect(listed.contains { $0.id == id })
        #expect(listed.first { $0.id == id }?.url == "https://example.com")
    }

    @Test("open without active project fails")
    func openNoProjectFails() {
        let appState = AppState(
            selectionStore: SelectionStoreStub(),
            terminalViews: TerminalViewRemovingStub(),
            workspacePersistence: WorkspacePersistenceStub()
        )
        UserDefaults.standard.removeObject(forKey: BrowserPreferences.enabledKey)
        let result = MuxyAPI.Browser.open(url: nil, appState: appState)
        #expect(result == .failure(.noActiveProject))
    }

    @Test("open fails when the browser is disabled")
    func openDisabledFails() {
        let appState = makeAppState()
        BrowserPreferences.isEnabled = false
        defer { UserDefaults.standard.removeObject(forKey: BrowserPreferences.enabledKey) }
        let result = MuxyAPI.Browser.open(url: "https://example.com", appState: appState)
        #expect(result == .failure(.browserDisabled))
    }

    @Test("navigate queues the url before a browser surface exists")
    func navigateQueuesURL() throws {
        let appState = makeAppState()
        let id = try MuxyAPI.Browser.open(url: "https://example.com", appState: appState).get()
        let result = MuxyAPI.Browser.navigate(tabIDString: id.uuidString, url: "muxy.app/docs", appState: appState)
        let state = try #require(
            appState.workspaceRoots.values
                .flatMap { $0.allTabs() }
                .first(where: { $0.id == id })?
                .content.browserState
        )

        #expect(isSuccess(result))
        #expect(state.pendingURL?.absoluteString == "https://muxy.app/docs")
    }

    @Test("navigate loads an inactive cached browser surface immediately")
    func navigateLoadsInactiveCachedSurface() throws {
        let appState = makeAppState()
        let id = try MuxyAPI.Browser.open(url: "https://example.com", appState: appState).get()
        let state = try #require(
            appState.workspaceRoots.values
                .flatMap { $0.allTabs() }
                .first(where: { $0.id == id })?
                .content.browserState
        )
        let webView = NavigationRecordingWebView(frame: .zero)
        state.webView = webView

        let result = MuxyAPI.Browser.navigate(
            tabIDString: id.uuidString,
            url: "muxy.app/docs",
            appState: appState
        )

        #expect(isSuccess(result))
        #expect(webView.lastRequest?.url?.absoluteString == "https://muxy.app/docs")
        #expect(state.pendingURL == nil)
    }

    @Test("navigate fails for an unknown tab id")
    func navigateUnknownFails() {
        let appState = makeAppState()
        let unknownID = UUID().uuidString
        let result = MuxyAPI.Browser.navigate(tabIDString: unknownID, url: "https://x.com", appState: appState)
        #expect(failureError(result) == .browserTabNotFound(unknownID))
    }

    private func isSuccess(_ result: Result<Void, APIError>) -> Bool {
        if case .success = result { return true }
        return false
    }

    private func failureError(_ result: Result<Void, APIError>) -> APIError? {
        if case let .failure(error) = result { return error }
        return nil
    }

    @Test("close removes the browser tab")
    func closeRemovesTab() throws {
        let appState = makeAppState()
        let id = try MuxyAPI.Browser.open(url: "https://example.com", appState: appState).get()
        _ = MuxyAPI.Browser.close(tabIDString: id.uuidString, appState: appState)
        let listed = MuxyAPI.Browser.list(appState: appState, profileStore: nil)
        #expect(!listed.contains { $0.id == id })
    }

    @Test("close fails when the browser is disabled")
    func closeDisabledFails() throws {
        let appState = makeAppState()
        let id = try MuxyAPI.Browser.open(url: "https://example.com", appState: appState).get()
        BrowserPreferences.isEnabled = false
        defer { UserDefaults.standard.removeObject(forKey: BrowserPreferences.enabledKey) }
        #expect(failureError(MuxyAPI.Browser.close(tabIDString: id.uuidString, appState: appState)) == .browserDisabled)
    }

    @Test("list resolves the profile name from the store")
    func listResolvesProfileName() throws {
        let appState = makeAppState()
        let store = BrowserProfileStore(persistence: BrowserProfilePersistenceStub())
        let profile = store.add(name: "Work")
        let id = try MuxyAPI.Browser.open(
            url: "https://example.com",
            profileID: profile.id,
            appState: appState
        ).get()
        let listed = MuxyAPI.Browser.list(appState: appState, profileStore: store)
        #expect(listed.first { $0.id == id }?.profile == "Work")
    }

    @Test("list returns empty when the browser is disabled")
    func listDisabledReturnsEmpty() throws {
        let appState = makeAppState()
        _ = try MuxyAPI.Browser.open(url: "https://example.com", appState: appState).get()
        BrowserPreferences.isEnabled = false
        defer { UserDefaults.standard.removeObject(forKey: BrowserPreferences.enabledKey) }
        #expect(MuxyAPI.Browser.list(appState: appState, profileStore: nil).isEmpty)
    }

    @Test("eval fails for an unknown tab id")
    func evalUnknownTabFails() async {
        let appState = makeAppState()
        let unknownID = UUID().uuidString
        let result = await MuxyAPI.Browser.eval(tabIDString: unknownID, script: "1+1", appState: appState)
        #expect(failureErrorString(result) == .browserTabNotFound(unknownID))
    }

    @Test("eval reports surface not ready for an unrendered tab")
    func evalUnrenderedTabNotReady() async throws {
        let appState = makeAppState()
        let id = try MuxyAPI.Browser.open(url: "https://example.com", appState: appState).get()
        let result = await MuxyAPI.Browser.eval(tabIDString: id.uuidString, script: "1+1", appState: appState)
        guard case let .failure(error) = result else {
            Issue.record("expected failure")
            return
        }
        guard case .browserTabSurfaceNotReady = error else {
            Issue.record("expected surface-not-ready, got \(error)")
            return
        }
    }

    @Test("automation verbs fail when the browser is disabled")
    func automationDisabledFails() async {
        let appState = makeAppState()
        BrowserPreferences.isEnabled = false
        defer { UserDefaults.standard.removeObject(forKey: BrowserPreferences.enabledKey) }
        let tabID = UUID().uuidString
        let eval = await MuxyAPI.Browser.eval(tabIDString: tabID, script: "1", appState: appState)
        let click = await MuxyAPI.Browser.click(tabIDString: tabID, selector: "a", appState: appState)
        let cookies = await MuxyAPI.Browser.cookiesClear(tabIDString: tabID, appState: appState)
        #expect(failureErrorString(eval) == .browserDisabled)
        #expect(failureErrorBool(click) == .browserDisabled)
        #expect(failureErrorVoid(cookies) == .browserDisabled)
    }

    @Test("jsString produces a quoted JS literal that escapes special characters")
    func jsStringEscapes() {
        #expect(jsString("a\"b") == "\"a\\\"b\"")
        #expect(jsString("a\\b") == "\"a\\\\b\"")
        #expect(jsString("a\nb") == "\"a\\nb\"")
        #expect(jsString("plain") == "\"plain\"")
    }

    @Test("jsString escapes Unicode line and paragraph separators")
    func jsStringEscapesLineSeparators() {
        #expect(jsString("a\u{2028}b") == "\"a\\u2028b\"")
        #expect(jsString("a\u{2029}b") == "\"a\\u2029b\"")
        #expect(!jsString("a\u{2028}b").unicodeScalars.contains("\u{2028}"))
        #expect(!jsString("a\u{2029}b").unicodeScalars.contains("\u{2029}"))
    }

    @Test("cookie fetches retire stalled requests and bound replacements")
    @MainActor
    func cookieFetchesRetireStalledRequests() async throws {
        let coordinator = CookieFetchCoordinator<Int>()
        let owner = CookieFetchOwner()
        let gate = CookieFetchGate()
        let replacementGate = CookieFetchGate()
        let counter = CookieFetchCounter()

        let first = Task { @MainActor in
            await coordinator.value(for: owner, timeout: .milliseconds(200)) {
                counter.increment()
                return await gate.value()
            }
        }
        let second = Task { @MainActor in
            await coordinator.value(for: owner, timeout: .milliseconds(200)) {
                counter.increment()
                return await gate.value()
            }
        }

        try #require(await eventually { coordinator.waiterCount == 2 && counter.value == 1 })
        let results = await [first.value, second.value]
        #expect(results == [nil, nil])
        #expect(counter.value == 1)
        #expect(coordinator.requestCount == 1)

        let recovered = Task { @MainActor in
            await coordinator.value(for: owner, timeout: .seconds(1)) {
                counter.increment()
                return await replacementGate.value()
            }
        }
        try #require(await eventually { coordinator.waiterCount == 1 && counter.value == 2 })
        #expect(coordinator.requestCount == 2)

        gate.resume(with: 1)
        try #require(await eventually { coordinator.requestCount == 1 })
        #expect(coordinator.waiterCount == 1)
        replacementGate.resume(with: 2)
        #expect(await recovered.value == 2)
        try #require(await eventually { coordinator.requestCount == 0 })

        let next = await coordinator.value(for: owner, timeout: .milliseconds(20)) {
            counter.increment()
            return 3
        }
        #expect(next == 3)
        #expect(counter.value == 3)
    }

    @Test("cookie fetches cap uncancellable stalled operations")
    @MainActor
    func cookieFetchesCapStalledOperations() async throws {
        let coordinator = CookieFetchCoordinator<Int>()
        let owner = CookieFetchOwner()
        let firstGate = CookieFetchGate()
        let secondGate = CookieFetchGate()
        let counter = CookieFetchCounter()

        let first = await coordinator.value(for: owner, timeout: .milliseconds(20)) {
            counter.increment()
            return await firstGate.value()
        }
        let second = await coordinator.value(for: owner, timeout: .milliseconds(20)) {
            counter.increment()
            return await secondGate.value()
        }
        let blocked = await coordinator.value(for: owner, timeout: .milliseconds(20)) {
            counter.increment()
            return 3
        }

        #expect(first == nil)
        #expect(second == nil)
        #expect(blocked == nil)
        #expect(counter.value == 2)
        #expect(coordinator.requestCount == 2)

        firstGate.resume(with: 1)
        secondGate.resume(with: 2)
        try #require(await eventually { coordinator.requestCount == 0 })
    }

    @MainActor
    private func eventually(_ condition: @MainActor () -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + .seconds(2)
        while ContinuousClock.now < deadline {
            if condition() { return true }
            try? await Task.sleep(for: .milliseconds(1))
        }
        return condition()
    }

    @Test("BrowserAutomation never uses main-thread blocking primitives")
    func automationHasNoBlockingPrimitives() throws {
        let url = RepositoryRoot.find()
            .appendingPathComponent("Muxy/Services/Browser/BrowserAutomation.swift")
        let source = try String(contentsOf: url, encoding: .utf8)
        let banned = [
            "DispatchSemaphore",
            ".wait()",
            "DispatchQueue.main.sync",
            "RunLoop.current.run",
            "RunLoop.main.run",
            "Thread.sleep",
        ]
        for token in banned {
            #expect(!source.contains(token), "BrowserAutomation.swift must not block the main thread with \(token)")
        }
    }

    private func failureErrorString(_ result: Result<String, APIError>) -> APIError? {
        if case let .failure(error) = result { return error }
        return nil
    }

    private func failureErrorBool(_ result: Result<Bool, APIError>) -> APIError? {
        if case let .failure(error) = result { return error }
        return nil
    }

    private func failureErrorVoid(_ result: Result<Void, APIError>) -> APIError? {
        if case let .failure(error) = result { return error }
        return nil
    }

    @Test("registry resolves a registered web view then clears on unregister")
    func registryRegistersAndUnregisters() {
        let registry = BrowserWebViewRegistry.shared
        let tabID = UUID()
        let webView = WKWebView(frame: .zero)
        registry.register(webView, for: tabID)
        #expect(registry.webView(for: tabID) === webView)
        registry.unregister(tabID)
        #expect(registry.webView(for: tabID) == nil)
    }

    @Test("screenshot reports surface not ready for an unrendered tab")
    func screenshotUnrenderedTabNotReady() async throws {
        let appState = makeAppState()
        let id = try MuxyAPI.Browser.open(url: "https://example.com", appState: appState).get()

        let result = await MuxyAPI.Browser.screenshot(tabIDString: id.uuidString, appState: appState)
        guard case let .failure(error) = result else {
            Issue.record("expected failure for an unrendered tab")
            return
        }
        guard case .browserTabSurfaceNotReady = error else {
            Issue.record("expected surface-not-ready, got \(error)")
            return
        }
    }

    @Test("screenshot fails for an unknown tab id")
    func screenshotUnknownTabFails() async {
        let appState = makeAppState()
        let unknownID = UUID().uuidString
        let result = await MuxyAPI.Browser.screenshot(tabIDString: unknownID, appState: appState)
        #expect(failureErrorString(result) == .browserTabNotFound(unknownID))
    }
}

@MainActor
private final class NavigationRecordingWebView: WKWebView {
    private(set) var lastRequest: URLRequest?

    override func load(_ request: URLRequest) -> WKNavigation? {
        lastRequest = request
        return nil
    }
}

private final class BrowserProfilePersistenceStub: BrowserProfilePersisting {
    private var stored: [BrowserProfile] = []
    func loadProfiles() throws -> [BrowserProfile] { stored }
    func saveProfiles(_ profiles: [BrowserProfile]) throws { stored = profiles }
}

private final class SelectionStoreStub: ActiveProjectSelectionStoring {
    private var activeProjectID: UUID?
    private var activeWorktreeIDs: [UUID: UUID] = [:]
    func loadActiveProjectID() -> UUID? { activeProjectID }
    func saveActiveProjectID(_ id: UUID?) { activeProjectID = id }
    func loadActiveWorktreeIDs() -> [UUID: UUID] { activeWorktreeIDs }
    func saveActiveWorktreeIDs(_ ids: [UUID: UUID]) { activeWorktreeIDs = ids }
}

@MainActor
private final class TerminalViewRemovingStub: TerminalViewRemoving {
    func removeView(for _: UUID) {}
    func needsConfirmQuit(for _: UUID) -> Bool { false }
}

private final class WorkspacePersistenceStub: WorkspacePersisting {
    func loadWorkspaces() throws -> [WorkspaceSnapshot] { [] }
    func saveWorkspaces(_: [WorkspaceSnapshot]) throws {}
}

@MainActor
private final class CookieFetchGate {
    private var continuation: CheckedContinuation<Int, Never>?

    func value() async -> Int {
        await withCheckedContinuation { continuation in
            self.continuation = continuation
        }
    }

    func resume(with value: Int) {
        continuation?.resume(returning: value)
        continuation = nil
    }
}

@MainActor
private final class CookieFetchOwner: NSObject {}

@MainActor
private final class CookieFetchCounter {
    private(set) var value = 0

    func increment() {
        value += 1
    }
}
