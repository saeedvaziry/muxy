import Foundation
import Testing

@testable import Muxy

@Suite("TerminalOfflineStore")
@MainActor
struct TerminalOfflineStoreTests {
    private let projectID = UUID()
    private let worktreeID = UUID()
    private let projectPath = "/tmp/project"

    @Test("worktree turns offline only once every terminal pane is offline")
    func worktreeOfflineWaitsForEveryPane() throws {
        let context = makeContext()
        let panes = paneStates(in: context.appState)
        #expect(panes.count == 2)

        panes[0].isOffline = true
        context.store.update(paneID: panes[0].id, appState: context.appState)
        #expect(context.store.state(for: context.worktreeKey) == false)
        #expect(context.recorder.events.isEmpty)

        panes[1].isOffline = true
        context.store.update(paneID: panes[1].id, appState: context.appState)
        #expect(context.store.state(for: context.worktreeKey) == true)
        #expect(context.recorder.events == [Emission(worktreeKey: context.worktreeKey, worktreePath: projectPath, offline: true)])
    }

    @Test("worktree turns online again once any terminal pane wakes")
    func worktreeOnlineWhenAnyPaneWakes() throws {
        let context = makeContext()
        let panes = paneStates(in: context.appState)

        for pane in panes {
            pane.isOffline = true
            context.store.update(paneID: pane.id, appState: context.appState)
        }
        panes[0].isOffline = false
        context.store.update(paneID: panes[0].id, appState: context.appState)

        #expect(context.store.state(for: context.worktreeKey) == false)
        #expect(context.recorder.events == [
            Emission(worktreeKey: context.worktreeKey, worktreePath: projectPath, offline: true),
            Emission(worktreeKey: context.worktreeKey, worktreePath: projectPath, offline: false),
        ])
    }

    @Test("offline event uses the worktree root when a pane starts in a subdirectory")
    func offlineEventUsesWorktreeRoot() throws {
        let panePath = "\(projectPath)/packages/app"
        let context = makeContext(additionalPanePath: panePath)
        let panes = paneStates(in: context.appState)
        let nestedPane = try #require(panes.first(where: { $0.projectPath == panePath }))

        for pane in panes where pane.id != nestedPane.id {
            pane.isOffline = true
            context.store.update(paneID: pane.id, appState: context.appState)
        }
        nestedPane.isOffline = true
        context.store.update(paneID: nestedPane.id, appState: context.appState)

        #expect(context.recorder.events == [
            Emission(worktreeKey: context.worktreeKey, worktreePath: projectPath, offline: true),
        ])
    }

    @Test("closing an online pane leaves an offline worktree behind")
    func closingOnlinePaneReportsOfflineWorktree() throws {
        let context = makeContext()
        let panes = paneStates(in: context.appState)

        panes[1].isOffline = true
        context.store.update(paneID: panes[1].id, appState: context.appState)
        context.store.removePane(panes[0].id)

        #expect(context.store.state(for: context.worktreeKey) == true)
        #expect(context.recorder.events == [Emission(worktreeKey: context.worktreeKey, worktreePath: projectPath, offline: true)])
    }

    @Test("closing the last pane of an offline worktree reports it online")
    func closingLastPaneReportsWorktreeOnline() throws {
        let context = makeContext()
        let panes = paneStates(in: context.appState)

        for pane in panes {
            pane.isOffline = true
            context.store.update(paneID: pane.id, appState: context.appState)
        }
        for pane in panes {
            context.store.removePane(pane.id)
        }

        #expect(context.store.state(for: context.worktreeKey) == nil)
        #expect(context.recorder.events == [
            Emission(worktreeKey: context.worktreeKey, worktreePath: projectPath, offline: true),
            Emission(worktreeKey: context.worktreeKey, worktreePath: projectPath, offline: false),
        ])
    }

    @Test("closing the last pane of an online worktree stays silent")
    func closingLastPaneOfOnlineWorktreeStaysSilent() throws {
        let context = makeContext()
        let panes = paneStates(in: context.appState)

        context.store.update(paneID: panes[0].id, appState: context.appState)
        for pane in panes {
            context.store.removePane(pane.id)
        }

        #expect(context.store.state(for: context.worktreeKey) == nil)
        #expect(context.recorder.events.isEmpty)
    }

    @Test("removing mixed-state panes together uses the final worktree state")
    func removingMixedStatePanesTogetherUsesFinalState() throws {
        let context = makeContext()
        let panes = paneStates(in: context.appState)

        panes[1].isOffline = true
        context.store.update(paneID: panes[1].id, appState: context.appState)
        context.store.removePanes([panes[0].id, panes[1].id])

        #expect(context.store.state(for: context.worktreeKey) == nil)
        #expect(context.recorder.events.isEmpty)
    }

    @Test("a pane created in an offline worktree reports it online")
    func createdPaneReportsWorktreeOnline() throws {
        let context = makeContext()
        let panes = paneStates(in: context.appState)

        for pane in panes {
            pane.isOffline = true
            context.store.update(paneID: pane.id, appState: context.appState)
        }
        context.store.addPane(UUID(), worktreeKey: context.worktreeKey, worktreePath: projectPath)

        #expect(context.store.state(for: context.worktreeKey) == false)
        #expect(context.recorder.events == [
            Emission(worktreeKey: context.worktreeKey, worktreePath: projectPath, offline: true),
            Emission(worktreeKey: context.worktreeKey, worktreePath: projectPath, offline: false),
        ])
    }

    @Test("a pane created in an untracked worktree stays silent")
    func createdPaneInUntrackedWorktreeStaysSilent() {
        let context = makeContext()

        context.store.addPane(UUID(), worktreeKey: context.worktreeKey, worktreePath: projectPath)

        #expect(context.store.state(for: context.worktreeKey) == false)
        #expect(context.recorder.events.isEmpty)
    }

    @Test("a pane created twice is registered once")
    func createdPaneIsRegisteredOnce() throws {
        let context = makeContext()
        let panes = paneStates(in: context.appState)
        let extraPaneID = UUID()

        for pane in panes {
            pane.isOffline = true
            context.store.update(paneID: pane.id, appState: context.appState)
        }
        context.store.addPane(extraPaneID, worktreeKey: context.worktreeKey, worktreePath: projectPath)
        context.store.addPane(extraPaneID, worktreeKey: context.worktreeKey, worktreePath: projectPath)

        #expect(context.store.state(for: context.worktreeKey) == false)
        #expect(context.recorder.events == [
            Emission(worktreeKey: context.worktreeKey, worktreePath: projectPath, offline: true),
            Emission(worktreeKey: context.worktreeKey, worktreePath: projectPath, offline: false),
        ])
    }

    private struct Emission: Equatable {
        let worktreeKey: WorktreeKey
        let worktreePath: String
        let offline: Bool
    }

    @MainActor
    private final class EmissionRecorder {
        private(set) var events: [Emission] = []

        func record(worktreeKey: WorktreeKey, worktreePath: String, offline: Bool) {
            events.append(Emission(worktreeKey: worktreeKey, worktreePath: worktreePath, offline: offline))
        }
    }

    private struct Context {
        let store: TerminalOfflineStore
        let appState: AppState
        let recorder: EmissionRecorder
        let worktreeKey: WorktreeKey
    }

    private func makeContext(additionalPanePath: String? = nil) -> Context {
        let appState = AppState(
            selectionStore: OfflineSelectionStoreStub(),
            terminalViews: OfflineTerminalViewRemovingStub(),
            workspacePersistence: OfflineWorkspacePersistenceStub()
        )
        let worktreeKey = WorktreeKey(projectID: projectID, worktreeID: worktreeID)
        let area = TabArea(projectPath: projectPath)
        appState.activeProjectID = projectID
        appState.activeWorktreeID[projectID] = worktreeID
        appState.workspaceRoots[worktreeKey] = .tabArea(area)
        appState.focusedAreaID[worktreeKey] = area.id
        if let additionalPanePath {
            _ = area.createTab(inDirectory: additionalPanePath)
        } else {
            _ = area.createTab()
        }

        let recorder = EmissionRecorder()
        let store = TerminalOfflineStore { key, path, offline in
            recorder.record(worktreeKey: key, worktreePath: path, offline: offline)
        }
        return Context(store: store, appState: appState, recorder: recorder, worktreeKey: worktreeKey)
    }

    private func paneStates(in appState: AppState) -> [TerminalPaneState] {
        appState.workspaceRoots.values
            .flatMap { $0.allAreas() }
            .flatMap(\.tabs)
            .compactMap(\.content.pane)
    }
}

private final class OfflineWorkspacePersistenceStub: WorkspacePersisting {
    func loadWorkspaces() throws -> [WorkspaceSnapshot] { [] }
    func saveWorkspaces(_: [WorkspaceSnapshot]) throws {}
}

@MainActor
private final class OfflineSelectionStoreStub: ActiveProjectSelectionStoring {
    private var activeProjectID: UUID?
    private var activeWorktreeIDs: [UUID: UUID] = [:]
    func loadActiveProjectID() -> UUID? { activeProjectID }
    func saveActiveProjectID(_ id: UUID?) { activeProjectID = id }
    func loadActiveWorktreeIDs() -> [UUID: UUID] { activeWorktreeIDs }
    func saveActiveWorktreeIDs(_ ids: [UUID: UUID]) { activeWorktreeIDs = ids }
}

@MainActor
private final class OfflineTerminalViewRemovingStub: TerminalViewRemoving {
    func removeView(for _: UUID) {}
    func needsConfirmQuit(for _: UUID) -> Bool { false }
}
