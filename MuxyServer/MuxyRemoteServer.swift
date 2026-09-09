import Foundation
import MuxyShared
import Network
import os

private let logger = Logger(subsystem: "app.muxy", category: "RemoteServer")

public enum DeviceAuthDecision: Sendable {
    case approved(deviceName: String)
    case unknown
    case denied
}

public enum MuxyRemoteServerError: LocalizedError {
    case invalidPort(UInt16)
    case startSuperseded

    public var errorDescription: String? {
        switch self {
        case let .invalidPort(port):
            "Invalid port \(port)."
        case .startSuperseded:
            "Server start was superseded by a new start request."
        }
    }
}

@MainActor
public protocol MuxyRemoteServerDelegate: AnyObject {
    func listProjects() -> [ProjectDTO]
    func listWorkspaces() -> [WorkspaceInfoDTO]
    func listProjectsByWorkspace(workspaceID: UUID) -> [ProjectDTO]
    func selectProject(_ projectID: UUID)
    func listWorktrees(projectID: UUID) -> [WorktreeDTO]
    func selectWorktree(projectID: UUID, worktreeID: UUID)
    func getWorkspace(projectID: UUID) -> WorkspaceDTO?
    func createTab(projectID: UUID, areaID: UUID?, kind: TabKindDTO) -> TabDTO?
    func closeTab(projectID: UUID, areaID: UUID, tabID: UUID)
    func selectTab(projectID: UUID, areaID: UUID, tabID: UUID)
    func splitArea(projectID: UUID, areaID: UUID, direction: SplitDirectionDTO, position: SplitPositionDTO)
    func closeArea(projectID: UUID, areaID: UUID)
    func focusArea(projectID: UUID, areaID: UUID)
    func sendTerminalInput(paneID: UUID, bytes: Data, clientID: UUID)
    func resizeTerminal(paneID: UUID, cols: UInt32, rows: UInt32, clientID: UUID)
    func scrollTerminal(paneID: UUID, deltaX: Double, deltaY: Double, precise: Bool, clientID: UUID)
    func getTerminalContent(paneID: UUID) -> TerminalCellsDTO?
    func takeOverPane(paneID: UUID, clientID: UUID, cols: UInt32, rows: UInt32)
    func releasePane(paneID: UUID, clientID: UUID)
    func setClientTheme(_ theme: ClientThemeDTO?, clientID: UUID)
    func registerDevice(clientID: UUID, name: String)
    func authenticateDevice(deviceID: UUID, token: String, name: String) -> DeviceAuthDecision
    func requestPairing(deviceID: UUID, token: String, name: String) async -> DeviceAuthDecision
    func getDeviceTheme() -> DeviceThemeEventDTO?
    func clientDisconnected(clientID: UUID)
    func getPaneOwner(paneID: UUID) -> PaneOwnerDTO?
    func getVCSStatus(projectID: UUID) async -> VCSStatusDTO?
    func vcsRefresh(projectID: UUID) async -> VCSStatusDTO?
    func vcsCommit(projectID: UUID, message: String, stageAll: Bool) async throws
    func vcsPush(projectID: UUID) async throws
    func vcsPull(projectID: UUID) async throws
    func vcsStageFiles(projectID: UUID, paths: [String]) async throws
    func vcsUnstageFiles(projectID: UUID, paths: [String]) async throws
    func vcsDiscardFiles(projectID: UUID, paths: [String], untrackedPaths: [String]) async throws
    func vcsListBranches(projectID: UUID) async throws -> VCSBranchesDTO
    func vcsSwitchBranch(projectID: UUID, branch: String) async throws
    func vcsCreateBranch(projectID: UUID, name: String) async throws
    func vcsCreatePR(projectID: UUID, title: String, body: String, baseBranch: String?, draft: Bool) async throws -> VCSCreatePRResultDTO
    func vcsMergePullRequest(projectID: UUID, number: Int, method: VCSMergeMethodDTO, deleteBranch: Bool) async throws
    func vcsAddWorktree(
        projectID: UUID,
        name: String,
        branch: String,
        createBranch: Bool,
        baseBranch: String?
    ) async throws -> WorktreeDTO
    func vcsRemoveWorktree(projectID: UUID, worktreeID: UUID) async throws
    func vcsGetDiff(projectID: UUID, filePath: String, forceFull: Bool) async throws -> VCSDiffDTO
    func filesList(projectID: UUID, path: String) async throws -> [FileEntryDTO]
    func filesRead(projectID: UUID, path: String, encoding: FileEncodingDTO) async throws -> FileContentDTO
    func filesStat(projectID: UUID, path: String) async throws -> FileStatDTO
    func filesWrite(projectID: UUID, path: String, contents: String, encoding: FileEncodingDTO) async throws -> String
    func filesMkdir(projectID: UUID, path: String) async throws -> String
    func filesRename(projectID: UUID, path: String, newName: String) async throws -> String
    func filesMove(projectID: UUID, paths: [String], into destination: String) async throws -> [String]
    func filesDelete(projectID: UUID, paths: [String]) async throws
    func getProjectLogo(projectID: UUID) -> ProjectLogoDTO?
    func listNotifications() -> [NotificationDTO]
    func markNotificationRead(_ notificationID: UUID)
    func extensionRequest(extension: String, action: String, payload: MuxyJSON, clientID: UUID) async -> Result<MuxyJSON, MuxyError>
}

public final class MuxyRemoteServer: @unchecked Sendable {
    public static let defaultPort: UInt16 = 4865
    public static let bonjourServiceType: String = "_muxy._tcp"

    private let port: UInt16
    private var listener: NWListener?
    private var connections: [UUID: ClientConnection] = [:]
    private var authenticatedClients: Set<UUID> = []
    private var deviceIDByClient: [UUID: UUID] = [:]
    private let queue = DispatchQueue(label: "app.muxy.remoteServer")
    private var startCompletion: (@Sendable (Result<Void, Error>) -> Void)?
    private var stopCompletions: [@Sendable () -> Void] = []
    public weak var delegate: (any MuxyRemoteServerDelegate)?

    public init(port: UInt16 = MuxyRemoteServer.defaultPort) {
        self.port = port
    }

    public func start(completion: (@Sendable (Result<Void, Error>) -> Void)? = nil) {
        queue.async { [weak self] in
            guard let self else { return }
            self.finishStart(.failure(MuxyRemoteServerError.startSuperseded))
            self.startCompletion = completion
            self.startListener()
        }
    }

    public func stop(completion: (@Sendable () -> Void)? = nil) {
        queue.async { [weak self] in
            guard let self else {
                completion?()
                return
            }
            for connection in self.connections.values {
                connection.cancel()
            }
            self.connections.removeAll()
            self.authenticatedClients.removeAll()
            self.deviceIDByClient.removeAll()

            guard let listener = self.listener else {
                logger.info("Remote server stopped")
                completion?()
                return
            }
            if let completion {
                self.stopCompletions.append(completion)
            }
            listener.cancel()
        }
    }

    public func broadcast(_ event: MuxyEvent) {
        guard let data = try? MuxyCodec.encode(.event(event)) else { return }
        queue.async { [weak self] in
            guard let self else { return }
            for clientID in self.authenticatedClients {
                self.connections[clientID]?.send(data)
            }
        }
    }

    public func send(_ event: MuxyEvent, to clientID: UUID) {
        guard let data = try? MuxyCodec.encode(.event(event)) else { return }
        queue.async { [weak self] in
            guard let self,
                  self.authenticatedClients.contains(clientID)
            else { return }
            self.connections[clientID]?.send(data)
        }
    }

    public func disconnect(clientID: UUID) {
        queue.async { [weak self] in
            self?.connections[clientID]?.cancel()
        }
    }

    public func disconnect(deviceID: UUID) {
        queue.async { [weak self] in
            guard let self else { return }
            let clientIDs = self.deviceIDByClient.filter { $0.value == deviceID }.map(\.key)
            for clientID in clientIDs {
                self.connections[clientID]?.cancel()
            }
        }
    }

    private func startListener() {
        guard let endpointPort = NWEndpoint.Port(rawValue: port) else {
            logger.error("Invalid port: \(self.port)")
            finishStart(.failure(MuxyRemoteServerError.invalidPort(port)))
            return
        }

        do {
            let params = NWParameters.tcp
            params.allowLocalEndpointReuse = true
            let ws = NWProtocolWebSocket.Options()
            ws.autoReplyPing = true
            params.defaultProtocolStack.applicationProtocols.insert(ws, at: 0)
            listener = try NWListener(using: params, on: endpointPort)
            listener?.service = NWListener.Service(name: nil, type: Self.bonjourServiceType)
        } catch {
            logger.error("Failed to create listener: \(error)")
            finishStart(.failure(error))
            return
        }

        listener?.stateUpdateHandler = { [weak self] state in
            guard let self else { return }
            switch state {
            case .ready:
                logger.info("Remote server listening on port \(self.port)")
                self.finishStart(.success(()))
            case let .failed(error):
                logger.error("Listener failed: \(error)")
                self.finishStart(.failure(error))
                self.listener?.cancel()
            case .cancelled:
                self.listener = nil
                logger.info("Remote server stopped")
                let completions = self.stopCompletions
                self.stopCompletions.removeAll()
                for completion in completions {
                    completion()
                }
            default:
                break
            }
        }

        listener?.newConnectionHandler = { [weak self] nwConnection in
            self?.handleNewConnection(nwConnection)
        }

        listener?.start(queue: queue)
    }

    private func finishStart(_ result: Result<Void, Error>) {
        guard let completion = startCompletion else { return }
        startCompletion = nil
        completion(result)
    }

    private func handleNewConnection(_ nwConnection: NWConnection) {
        let id = UUID()
        let connection = ClientConnection(id: id, connection: nwConnection, server: self)
        connections[id] = connection
        connection.start(on: queue)
        logger.info("Client connected: \(id)")
    }

    func removeConnection(_ id: UUID) {
        queue.async { [weak self] in
            self?.connections.removeValue(forKey: id)
            self?.authenticatedClients.remove(id)
            self?.deviceIDByClient.removeValue(forKey: id)
            logger.info("Client disconnected: \(id)")
        }
        Task { @MainActor in
            self.delegate?.clientDisconnected(clientID: id)
        }
    }

    private func markAuthenticated(_ id: UUID, deviceID: UUID) {
        queue.async { [weak self] in
            self?.authenticatedClients.insert(id)
            self?.deviceIDByClient[id] = deviceID
        }
    }

    func _testingMarkAuthenticated(_ id: UUID) {
        queue.sync {
            _ = authenticatedClients.insert(id)
        }
    }

    private func isAuthenticated(_ id: UUID) -> Bool {
        queue.sync { authenticatedClients.contains(id) }
    }

    func handleRequest(_ request: MuxyRequest, from clientID: UUID) {
        if request.method == .takeOverPane {
            Task { @MainActor in
                self.processTakeOverRequest(request, clientID: clientID) { response in
                    self.send(response, to: clientID)
                }
            }
            return
        }
        if Self.voidMethods.contains(request.method) {
            Task { @MainActor in _ = await processRequest(request, clientID: clientID) }
            return
        }
        Task { @MainActor in
            let response = await processRequest(request, clientID: clientID)
            self.send(response, to: clientID)
        }
    }

    private func send(_ response: MuxyResponse, to clientID: UUID) {
        guard let data = try? MuxyCodec.encode(.response(response)) else { return }
        queue.async { [weak self] in
            self?.connections[clientID]?.send(data)
        }
    }

    @MainActor
    func processTakeOverRequest(
        _ request: MuxyRequest,
        clientID: UUID,
        responseHandler: (MuxyResponse) -> Void
    ) {
        guard let delegate else {
            responseHandler(MuxyResponse(id: request.id, error: .internalError))
            return
        }
        guard isAuthenticated(clientID) else {
            responseHandler(MuxyResponse(id: request.id, error: .unauthorized))
            return
        }
        guard case let .takeOverPane(params) = request.params else {
            responseHandler(MuxyResponse(id: request.id, error: .invalidParams))
            return
        }

        responseHandler(MuxyResponse(id: request.id, result: .ok))
        delegate.takeOverPane(
            paneID: params.paneID,
            clientID: clientID,
            cols: params.cols,
            rows: params.rows
        )
    }

    private static let voidMethods: Set<MuxyMethod> = [.terminalInput]

    @MainActor
    func processRequest(_ request: MuxyRequest, clientID: UUID) async -> MuxyResponse {
        guard let delegate else {
            return MuxyResponse(id: request.id, error: MuxyError.internalError)
        }

        switch request.method {
        case .pairDevice:
            guard case let .pairDevice(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            let decision = await delegate.requestPairing(
                deviceID: params.deviceID,
                token: params.token,
                name: params.deviceName
            )
            return finalizeAuth(
                requestID: request.id,
                clientID: clientID,
                deviceID: params.deviceID,
                decision: decision,
                clientTheme: params.theme
            )

        case .authenticateDevice:
            guard case let .authenticateDevice(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            let decision = delegate.authenticateDevice(
                deviceID: params.deviceID,
                token: params.token,
                name: params.deviceName
            )
            return finalizeAuth(
                requestID: request.id,
                clientID: clientID,
                deviceID: params.deviceID,
                decision: decision,
                clientTheme: params.theme
            )

        default:
            break
        }

        guard isAuthenticated(clientID) else {
            return MuxyResponse(id: request.id, error: .unauthorized)
        }

        switch request.method {
        case .pairDevice,
             .authenticateDevice:
            return MuxyResponse(id: request.id, error: .internalError)

        case .listProjects:
            let projects = delegate.listProjects()
            return MuxyResponse(id: request.id, result: .projects(projects))

        case .listWorkspaces:
            let workspaces = delegate.listWorkspaces()
            return MuxyResponse(id: request.id, result: .workspaces(workspaces))

        case .listProjectsByWorkspace:
            guard case let .listProjectsByWorkspace(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            let projects = delegate.listProjectsByWorkspace(workspaceID: params.workspaceID)
            return MuxyResponse(id: request.id, result: .projects(projects))

        case .selectProject:
            guard case let .selectProject(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            delegate.selectProject(params.projectID)
            return MuxyResponse(id: request.id, result: .ok)

        case .listWorktrees:
            guard case let .listWorktrees(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            let worktrees = delegate.listWorktrees(projectID: params.projectID)
            return MuxyResponse(id: request.id, result: .worktrees(worktrees))

        case .selectWorktree:
            guard case let .selectWorktree(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            delegate.selectWorktree(projectID: params.projectID, worktreeID: params.worktreeID)
            return MuxyResponse(id: request.id, result: .ok)

        case .getWorkspace:
            guard case let .getWorkspace(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            guard let workspace = delegate.getWorkspace(projectID: params.projectID) else {
                return MuxyResponse(id: request.id, error: .notFound)
            }
            return MuxyResponse(id: request.id, result: .workspace(workspace))

        case .createTab:
            guard case let .createTab(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            guard let tab = delegate.createTab(projectID: params.projectID, areaID: params.areaID, kind: params.kind) else {
                return MuxyResponse(id: request.id, error: .internalError)
            }
            return MuxyResponse(id: request.id, result: .tab(tab))

        case .closeTab:
            guard case let .closeTab(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            delegate.closeTab(projectID: params.projectID, areaID: params.areaID, tabID: params.tabID)
            return MuxyResponse(id: request.id, result: .ok)

        case .selectTab:
            guard case let .selectTab(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            delegate.selectTab(projectID: params.projectID, areaID: params.areaID, tabID: params.tabID)
            return MuxyResponse(id: request.id, result: .ok)

        case .splitArea:
            guard case let .splitArea(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            delegate.splitArea(
                projectID: params.projectID,
                areaID: params.areaID,
                direction: params.direction,
                position: params.position
            )
            return MuxyResponse(id: request.id, result: .ok)

        case .closeArea:
            guard case let .closeArea(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            delegate.closeArea(projectID: params.projectID, areaID: params.areaID)
            return MuxyResponse(id: request.id, result: .ok)

        case .focusArea:
            guard case let .focusArea(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            delegate.focusArea(projectID: params.projectID, areaID: params.areaID)
            return MuxyResponse(id: request.id, result: .ok)

        case .terminalInput:
            guard case let .terminalInput(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            delegate.sendTerminalInput(paneID: params.paneID, bytes: params.bytes, clientID: clientID)
            return MuxyResponse(id: request.id, result: .ok)

        case .terminalResize:
            guard case let .terminalResize(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            delegate.resizeTerminal(
                paneID: params.paneID,
                cols: params.cols,
                rows: params.rows,
                clientID: clientID
            )
            return MuxyResponse(id: request.id, result: .ok)

        case .terminalScroll:
            guard case let .terminalScroll(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            delegate.scrollTerminal(
                paneID: params.paneID,
                deltaX: params.deltaX,
                deltaY: params.deltaY,
                precise: params.precise,
                clientID: clientID
            )
            return MuxyResponse(id: request.id, result: .ok)

        case .getTerminalContent:
            guard case let .getTerminalContent(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            guard let content = delegate.getTerminalContent(paneID: params.paneID) else {
                return MuxyResponse(id: request.id, error: .notFound)
            }
            return MuxyResponse(id: request.id, result: .terminalCells(content))

        case .getVCSStatus:
            guard case let .getVCSStatus(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            guard let status = await delegate.getVCSStatus(projectID: params.projectID) else {
                return MuxyResponse(id: request.id, error: .notFound)
            }
            return MuxyResponse(id: request.id, result: .vcsStatus(status))

        case .vcsRefresh:
            guard case let .vcsRefresh(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            guard let status = await delegate.vcsRefresh(projectID: params.projectID) else {
                return MuxyResponse(id: request.id, error: .notFound)
            }
            return MuxyResponse(id: request.id, result: .vcsStatus(status))

        case .vcsCommit:
            guard case let .vcsCommit(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                try await delegate.vcsCommit(projectID: params.projectID, message: params.message, stageAll: params.stageAll)
                return MuxyResponse(id: request.id, result: .ok)
            } catch {
                return MuxyResponse(id: request.id, error: MuxyError(code: 500, message: error.localizedDescription))
            }

        case .vcsPush:
            guard case let .vcsPush(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                try await delegate.vcsPush(projectID: params.projectID)
                return MuxyResponse(id: request.id, result: .ok)
            } catch {
                return MuxyResponse(id: request.id, error: MuxyError(code: 500, message: error.localizedDescription))
            }

        case .vcsPull:
            guard case let .vcsPull(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                try await delegate.vcsPull(projectID: params.projectID)
                return MuxyResponse(id: request.id, result: .ok)
            } catch {
                return MuxyResponse(id: request.id, error: MuxyError(code: 500, message: error.localizedDescription))
            }

        case .vcsStageFiles:
            guard case let .vcsStageFiles(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                try await delegate.vcsStageFiles(projectID: params.projectID, paths: params.paths)
                return MuxyResponse(id: request.id, result: .ok)
            } catch {
                return MuxyResponse(id: request.id, error: MuxyError(code: 500, message: error.localizedDescription))
            }

        case .vcsUnstageFiles:
            guard case let .vcsUnstageFiles(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                try await delegate.vcsUnstageFiles(projectID: params.projectID, paths: params.paths)
                return MuxyResponse(id: request.id, result: .ok)
            } catch {
                return MuxyResponse(id: request.id, error: MuxyError(code: 500, message: error.localizedDescription))
            }

        case .vcsDiscardFiles:
            guard case let .vcsDiscardFiles(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                try await delegate.vcsDiscardFiles(
                    projectID: params.projectID,
                    paths: params.paths,
                    untrackedPaths: params.untrackedPaths
                )
                return MuxyResponse(id: request.id, result: .ok)
            } catch {
                return MuxyResponse(id: request.id, error: MuxyError(code: 500, message: error.localizedDescription))
            }

        case .vcsListBranches:
            guard case let .vcsListBranches(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                let branches = try await delegate.vcsListBranches(projectID: params.projectID)
                return MuxyResponse(id: request.id, result: .vcsBranches(branches))
            } catch {
                return MuxyResponse(id: request.id, error: MuxyError(code: 500, message: error.localizedDescription))
            }

        case .vcsSwitchBranch:
            guard case let .vcsSwitchBranch(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                try await delegate.vcsSwitchBranch(projectID: params.projectID, branch: params.branch)
                return MuxyResponse(id: request.id, result: .ok)
            } catch {
                return MuxyResponse(id: request.id, error: MuxyError(code: 500, message: error.localizedDescription))
            }

        case .vcsCreateBranch:
            guard case let .vcsCreateBranch(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                try await delegate.vcsCreateBranch(projectID: params.projectID, name: params.name)
                return MuxyResponse(id: request.id, result: .ok)
            } catch {
                return MuxyResponse(id: request.id, error: MuxyError(code: 500, message: error.localizedDescription))
            }

        case .vcsCreatePR:
            guard case let .vcsCreatePR(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                let info = try await delegate.vcsCreatePR(
                    projectID: params.projectID,
                    title: params.title,
                    body: params.body,
                    baseBranch: params.baseBranch,
                    draft: params.draft
                )
                return MuxyResponse(id: request.id, result: .vcsPRCreated(info))
            } catch {
                return MuxyResponse(id: request.id, error: MuxyError(code: 500, message: error.localizedDescription))
            }

        case .vcsMergePullRequest:
            guard case let .vcsMergePullRequest(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                try await delegate.vcsMergePullRequest(
                    projectID: params.projectID,
                    number: params.number,
                    method: params.method,
                    deleteBranch: params.deleteBranch
                )
                return MuxyResponse(id: request.id, result: .ok)
            } catch {
                return MuxyResponse(id: request.id, error: MuxyError(code: 500, message: error.localizedDescription))
            }

        case .vcsAddWorktree:
            guard case let .vcsAddWorktree(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                let worktree = try await delegate.vcsAddWorktree(
                    projectID: params.projectID,
                    name: params.name,
                    branch: params.branch,
                    createBranch: params.createBranch,
                    baseBranch: params.baseBranch
                )
                return MuxyResponse(id: request.id, result: .worktrees([worktree]))
            } catch {
                return MuxyResponse(id: request.id, error: MuxyError(code: 500, message: error.localizedDescription))
            }

        case .vcsRemoveWorktree:
            guard case let .vcsRemoveWorktree(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                try await delegate.vcsRemoveWorktree(projectID: params.projectID, worktreeID: params.worktreeID)
                return MuxyResponse(id: request.id, result: .ok)
            } catch {
                return MuxyResponse(id: request.id, error: MuxyError(code: 500, message: error.localizedDescription))
            }

        case .vcsGetDiff:
            guard case let .vcsGetDiff(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                let diff = try await delegate.vcsGetDiff(
                    projectID: params.projectID,
                    filePath: params.filePath,
                    forceFull: params.forceFull
                )
                return MuxyResponse(id: request.id, result: .vcsDiff(diff))
            } catch {
                return MuxyResponse(id: request.id, error: MuxyError(code: 500, message: error.localizedDescription))
            }

        case .filesList:
            guard case let .filesList(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                let entries = try await delegate.filesList(projectID: params.projectID, path: params.path)
                return MuxyResponse(id: request.id, result: .files(entries))
            } catch {
                return Self.failure(requestID: request.id, error: error)
            }

        case .filesRead:
            guard case let .filesRead(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                let content = try await delegate.filesRead(
                    projectID: params.projectID,
                    path: params.path,
                    encoding: params.encoding
                )
                return MuxyResponse(id: request.id, result: .fileContent(content))
            } catch {
                return Self.failure(requestID: request.id, error: error)
            }

        case .filesStat:
            guard case let .filesStat(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                let stat = try await delegate.filesStat(projectID: params.projectID, path: params.path)
                return MuxyResponse(id: request.id, result: .fileStat(stat))
            } catch {
                return Self.failure(requestID: request.id, error: error)
            }

        case .filesWrite:
            guard case let .filesWrite(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                let path = try await delegate.filesWrite(
                    projectID: params.projectID,
                    path: params.path,
                    contents: params.contents,
                    encoding: params.encoding
                )
                return MuxyResponse(id: request.id, result: .filePaths([path]))
            } catch {
                return Self.failure(requestID: request.id, error: error)
            }

        case .filesMkdir:
            guard case let .filesMkdir(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                let path = try await delegate.filesMkdir(projectID: params.projectID, path: params.path)
                return MuxyResponse(id: request.id, result: .filePaths([path]))
            } catch {
                return Self.failure(requestID: request.id, error: error)
            }

        case .filesRename:
            guard case let .filesRename(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                let path = try await delegate.filesRename(
                    projectID: params.projectID,
                    path: params.path,
                    newName: params.newName
                )
                return MuxyResponse(id: request.id, result: .filePaths([path]))
            } catch {
                return Self.failure(requestID: request.id, error: error)
            }

        case .filesMove:
            guard case let .filesMove(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                let paths = try await delegate.filesMove(
                    projectID: params.projectID,
                    paths: params.paths,
                    into: params.into
                )
                return MuxyResponse(id: request.id, result: .filePaths(paths))
            } catch {
                return Self.failure(requestID: request.id, error: error)
            }

        case .filesDelete:
            guard case let .filesDelete(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            do {
                try await delegate.filesDelete(projectID: params.projectID, paths: params.paths)
                return MuxyResponse(id: request.id, result: .ok)
            } catch {
                return Self.failure(requestID: request.id, error: error)
            }

        case .getProjectLogo:
            guard case let .getProjectLogo(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            guard let logo = delegate.getProjectLogo(projectID: params.projectID) else {
                return MuxyResponse(id: request.id, error: .notFound)
            }
            return MuxyResponse(id: request.id, result: .projectLogo(logo))

        case .listNotifications:
            let notifications = delegate.listNotifications()
            return MuxyResponse(id: request.id, result: .notifications(notifications))

        case .markNotificationRead:
            guard case let .markNotificationRead(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            delegate.markNotificationRead(params.notificationID)
            return MuxyResponse(id: request.id, result: .ok)

        case .subscribe,
             .unsubscribe:
            return MuxyResponse(id: request.id, result: .ok)

        case .registerDevice:
            guard case let .registerDevice(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            delegate.registerDevice(clientID: clientID, name: params.deviceName)
            let theme = delegate.getDeviceTheme()
            let info = DeviceInfoDTO(
                clientID: clientID,
                deviceName: params.deviceName,
                themeFg: theme?.fg,
                themeBg: theme?.bg,
                themePalette: theme?.palette
            )
            return MuxyResponse(id: request.id, result: .deviceInfo(info))

        case .takeOverPane:
            var response = MuxyResponse(id: request.id, error: .internalError)
            processTakeOverRequest(request, clientID: clientID) {
                response = $0
            }
            return response

        case .releasePane:
            guard case let .releasePane(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            delegate.releasePane(paneID: params.paneID, clientID: clientID)
            return MuxyResponse(id: request.id, result: .ok)

        case .setClientTheme:
            guard case let .setClientTheme(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            delegate.setClientTheme(params.theme, clientID: clientID)
            return MuxyResponse(id: request.id, result: .ok)

        case .extensionRequest:
            guard case let .extensionRequest(params) = request.params else {
                return MuxyResponse(id: request.id, error: .invalidParams)
            }
            let result = await delegate.extensionRequest(
                extension: params.extension,
                action: params.action,
                payload: params.payload,
                clientID: clientID
            )
            switch result {
            case let .success(payload):
                return MuxyResponse(id: request.id, result: .extensionResult(ExtensionResultDTO(payload: payload)))
            case let .failure(error):
                return MuxyResponse(id: request.id, error: error)
            }
        }
    }

    private static func failure(requestID: String, error: Error) -> MuxyResponse {
        guard let muxyError = error as? MuxyError else {
            return MuxyResponse(id: requestID, error: MuxyError(code: 500, message: error.localizedDescription))
        }
        return MuxyResponse(id: requestID, error: muxyError)
    }

    @MainActor
    private func finalizeAuth(
        requestID: String,
        clientID: UUID,
        deviceID: UUID,
        decision: DeviceAuthDecision,
        clientTheme: ClientThemeDTO?
    ) -> MuxyResponse {
        switch decision {
        case let .approved(deviceName):
            markAuthenticated(clientID, deviceID: deviceID)
            delegate?.registerDevice(clientID: clientID, name: deviceName)
            if let clientTheme {
                delegate?.setClientTheme(clientTheme, clientID: clientID)
            }
            let theme = delegate?.getDeviceTheme()
            let result = PairingResultDTO(
                clientID: clientID,
                deviceName: deviceName,
                themeFg: theme?.fg,
                themeBg: theme?.bg,
                themePalette: theme?.palette
            )
            return MuxyResponse(id: requestID, result: .pairing(result))
        case .unknown:
            return MuxyResponse(id: requestID, error: .unauthorized)
        case .denied:
            return MuxyResponse(id: requestID, error: .pairingDenied)
        }
    }
}
