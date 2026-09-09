import Foundation
import Testing

@testable import Muxy

@Suite("MuxyAPI workspace permissions")
struct MuxyAPIWorkspacePermissionTests {
    @Test("workspace verbs require projects:write", arguments: [
        "workspaces.create",
        "workspaces.switch",
        "workspaces.rename",
        "workspaces.delete",
        "projects.create",
        "projects.attach",
        "projects.detach",
    ])
    func workspaceVerbsRequireProjectsWrite(verb: String) {
        #expect(MuxyAPI.Permissions.required(for: verb) == .projectsWrite)
        #expect(MuxyAPI.Permissions.verbNames.contains(verb))
    }

    @Test("workspaces.list requires projects:read")
    func listRequiresProjectsRead() {
        #expect(MuxyAPI.Permissions.required(for: "workspaces.list") == .projectsRead)
        #expect(MuxyAPI.Permissions.verbNames.contains("workspaces.list"))
    }
}

@Suite("MuxyAPI workspace routing")
@MainActor
struct MuxyAPIWorkspaceRoutingTests {
    @Test("list returns empty when no workspaces exist")
    func listEmpty() {
        let store = makeStore()

        let result = MuxyAPI.Workspaces.list(projectGroupStore: store)

        #expect(result.isEmpty)
    }

    @Test("list returns workspace info with project count and active flag")
    func listReturnsInfo() {
        let group = ProjectGroup(name: "Work", projectIDs: [UUID(), UUID()])
        let store = makeStore(initial: [group], activeGroupID: group.id)

        let result = MuxyAPI.Workspaces.list(projectGroupStore: store)

        #expect(result.count == 1)
        #expect(result.first?.id == group.id)
        #expect(result.first?.name == "Work")
        #expect(result.first?.projectCount == 2)
        #expect(result.first?.isActive == true)
    }

    @Test("list counts the remote projects of an SSH workspace")
    func listCountsRemoteProjects() {
        let device = RemoteDevice(name: "Prod", ssh: SSHWorkspaceData(host: "example.com"))
        let store = ProjectGroupStore(
            persistence: ProjectGroupPersistenceStub(),
            remoteDeviceStore: RemoteDeviceStore(persistence: InMemoryRemoteDevicePersistence(initial: [device])),
            workspaceContextSink: InMemoryWorkspaceContextSink()
        )
        let group = store.addRemoteWorkspace(name: "Remote", deviceID: device.id)
        _ = store.addRemoteProject(name: "api", path: "/srv/api", toGroup: group.id)
        _ = store.addRemoteProject(name: "web", path: "/srv/web", toGroup: group.id)

        let result = MuxyAPI.Workspaces.list(projectGroupStore: store)

        #expect(result.first?.projectCount == 2)
    }

    @Test("create adds a workspace")
    func createAddsWorkspace() {
        let store = makeStore()

        let id = MuxyAPI.Workspaces.create(name: "Work", projectGroupStore: store)

        #expect(store.groups.count == 1)
        #expect(store.groups.first?.id == id)
        #expect(store.groups.first?.name == "Work")
    }

    @Test("create with empty name uses default")
    func createEmptyNameDefaults() {
        let store = makeStore()

        _ = MuxyAPI.Workspaces.create(name: "   ", projectGroupStore: store)

        #expect(store.groups.first?.name == "New Workspace")
    }

    @Test("create activates the new workspace")
    func createActivatesWorkspace() {
        let store = makeStore()

        let id = MuxyAPI.Workspaces.create(name: "Work", projectGroupStore: store)

        #expect(store.activeGroupID == id)
    }

    @Test("switchTo activates a workspace by name")
    func switchToByName() {
        let group = ProjectGroup(name: "Work")
        let store = makeStore(initial: [group])

        let result = MuxyAPI.Workspaces.switchTo(
            identifier: "work",
            projectGroupStore: store
        )

        #expect(isSuccess(result))
        #expect(store.activeGroupID == group.id)
    }

    @Test("switchTo activates a workspace by id")
    func switchToByID() {
        let group = ProjectGroup(name: "Work")
        let store = makeStore(initial: [group])

        let result = MuxyAPI.Workspaces.switchTo(
            identifier: group.id.uuidString,
            projectGroupStore: store
        )

        #expect(isSuccess(result))
        #expect(store.activeGroupID == group.id)
    }

    @Test("switchTo fails for unknown workspace")
    func switchToUnknown() {
        let store = makeStore()

        let result = MuxyAPI.Workspaces.switchTo(
            identifier: "missing",
            projectGroupStore: store
        )

        guard case .failure(.invalidArguments) = result else {
            Issue.record("expected invalidArguments for unknown workspace")
            return
        }
    }

    @Test("rename updates workspace name")
    func rename() {
        let group = ProjectGroup(name: "Work")
        let store = makeStore(initial: [group])

        let result = MuxyAPI.Workspaces.rename(
            identifier: "Work",
            to: "Personal",
            projectGroupStore: store
        )

        #expect(isSuccess(result))
        #expect(store.groups.first?.name == "Personal")
    }

    @Test("rename trims whitespace from the new name")
    func renameTrims() {
        let group = ProjectGroup(name: "Work")
        let store = makeStore(initial: [group])

        let result = MuxyAPI.Workspaces.rename(
            identifier: group.id.uuidString,
            to: "  Personal  ",
            projectGroupStore: store
        )

        #expect(isSuccess(result))
        #expect(store.groups.first?.name == "Personal")
    }

    @Test("rename rejects an empty name")
    func renameRejectsEmptyName() {
        let group = ProjectGroup(name: "Work")
        let store = makeStore(initial: [group])

        let result = MuxyAPI.Workspaces.rename(
            identifier: group.id.uuidString,
            to: "   ",
            projectGroupStore: store
        )

        guard case .failure(.invalidArguments) = result else {
            Issue.record("expected invalidArguments for empty name")
            return
        }
        #expect(store.groups.first?.name == "Work")
    }

    @Test("rename fails for unknown workspace")
    func renameUnknown() {
        let store = makeStore()

        let result = MuxyAPI.Workspaces.rename(
            identifier: "missing",
            to: "Personal",
            projectGroupStore: store
        )

        guard case .failure(.invalidArguments) = result else {
            Issue.record("expected invalidArguments for unknown workspace")
            return
        }
    }

    @Test("delete removes a workspace")
    func delete() {
        let group = ProjectGroup(name: "Work")
        let store = makeStore(initial: [group])

        let result = MuxyAPI.Workspaces.delete(
            identifier: "Work",
            projectGroupStore: store
        )

        #expect(isSuccess(result))
        #expect(store.groups.isEmpty)
    }

    @Test("delete fails for unknown workspace")
    func deleteUnknown() {
        let store = makeStore()

        let result = MuxyAPI.Workspaces.delete(
            identifier: "missing",
            projectGroupStore: store
        )

        guard case .failure(.invalidArguments) = result else {
            Issue.record("expected invalidArguments for unknown workspace")
            return
        }
    }

    @Test("delete fails when the workspace still contains projects")
    func deleteRejectsNonEmptyWorkspace() {
        let group = ProjectGroup(name: "Work", projectIDs: [UUID()])
        let store = makeStore(initial: [group])

        let result = MuxyAPI.Workspaces.delete(
            identifier: "Work",
            projectGroupStore: store
        )

        guard case .failure(.invalidArguments) = result else {
            Issue.record("expected invalidArguments for a non-empty workspace")
            return
        }
        #expect(store.groups.count == 1)
    }

    private func isSuccess(_ result: Result<some Any, APIError>) -> Bool {
        if case .success = result { return true }
        return false
    }

    private func makeStore(
        initial: [ProjectGroup] = [],
        activeGroupID: UUID? = nil
    ) -> ProjectGroupStore {
        ProjectGroupStore(
            persistence: ProjectGroupPersistenceStub(
                initial: initial,
                storedActiveGroupID: activeGroupID
            ),
            remoteDeviceStore: RemoteDeviceStore(persistence: InMemoryRemoteDevicePersistence()),
            workspaceContextSink: InMemoryWorkspaceContextSink()
        )
    }
}
