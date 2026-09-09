import Foundation
import os
import Security

private let logger = Logger(subsystem: "app.muxy", category: "ExtensionStore")

protocol ExtensionSnapshotSink: Sendable {
    func applyExtensionSnapshot(_ snapshot: NotificationSocketServer.ExtensionSnapshot)
    func applyExtensionSnapshotSync(_ snapshot: NotificationSocketServer.ExtensionSnapshot)
}

extension ExtensionSnapshotSink {
    func applyExtensionSnapshotSync(_ snapshot: NotificationSocketServer.ExtensionSnapshot) {
        applyExtensionSnapshot(snapshot)
    }
}

extension NotificationSocketServer: ExtensionSnapshotSink {}

@MainActor
@Observable
final class ExtensionStore {
    static let shared = ExtensionStore()

    struct ExtensionStatus: Identifiable, Equatable {
        let id: String
        let muxyExtension: MuxyExtension
        var isEnabled: Bool
        var isRunning: Bool
        var isDev: Bool
        var devSourcePath: String?
        var lastError: String?

        var logFileURL: URL {
            ExtensionLogStore.shared.logURL(
                extensionID: id,
                directory: muxyExtension.directory
            )
        }
    }

    struct LoadFailure: Identifiable, Equatable {
        let id = UUID()
        let directory: URL
        let message: String
        var devSourcePath: String?
    }

    private(set) var statuses: [ExtensionStatus] = []
    private(set) var loadFailures: [LoadFailure] = []
    private(set) var availableUpdates: [String: String] = [:]
    private(set) var hasLoadedFromDisk = false

    private var processes: [String: Process] = [:]
    private var hostPGIDs: [String: pid_t] = [:]
    var spawnedHostPGIDs: Set<pid_t> { Set(hostPGIDs.values) }
    private var tokens: [String: String] = [:]
    private var intentionalStops: Set<String> = []
    private var crashRestartAttempts: [String: Int] = [:]
    private var crashRestartTasks: [String: Task<Void, Never>] = [:]
    private let rootDirectoryURL: URL
    private let snapshotSink: ExtensionSnapshotSink
    private let resolveHostURL: @MainActor () -> URL?
    private let marketplace: ExtensionMarketplaceService
    private let devPathsProvider: @MainActor () -> [String]
    private let localizationsDidChange: @MainActor (ExtensionStore) -> Void

    nonisolated private static let processTerminationGracePeriod: TimeInterval = 2
    nonisolated private static let maxCrashRestartAttempts = 5
    nonisolated private static let crashRestartBackoff: TimeInterval = 1
    nonisolated private static let crashStabilityWindow: TimeInterval = 30

    private init(
        rootDirectory: URL = ExtensionStore.defaultRootDirectory,
        snapshotSink: ExtensionSnapshotSink = NotificationSocketServer.shared,
        resolveHostURL: @escaping @MainActor () -> URL? = { ExtensionHostLocator.hostURL() },
        marketplace: ExtensionMarketplaceService = .shared,
        devPathsProvider: @escaping @MainActor () -> [String] = { ExtensionDevPathStore.paths() },
        localizationsDidChange: @escaping @MainActor (ExtensionStore) -> Void = {
            LocalizationService.shared.refresh(store: $0)
        }
    ) {
        rootDirectoryURL = rootDirectory
        self.snapshotSink = snapshotSink
        self.resolveHostURL = resolveHostURL
        self.marketplace = marketplace
        self.devPathsProvider = devPathsProvider
        self.localizationsDidChange = localizationsDidChange
    }

    static func makeForTesting(
        rootDirectory: URL,
        snapshotSink: ExtensionSnapshotSink,
        resolveHostURL: @escaping @MainActor () -> URL?,
        marketplace: ExtensionMarketplaceService = .shared,
        devPathsProvider: @escaping @MainActor () -> [String] = { [] },
        localizationsDidChange: @escaping @MainActor (ExtensionStore) -> Void = { _ in }
    ) -> ExtensionStore {
        ExtensionStore(
            rootDirectory: rootDirectory,
            snapshotSink: snapshotSink,
            resolveHostURL: resolveHostURL,
            marketplace: marketplace,
            devPathsProvider: devPathsProvider,
            localizationsDidChange: localizationsDidChange
        )
    }

    var hasUpdates: Bool { !availableUpdates.isEmpty }
    var updateCount: Int { availableUpdates.count }

    func hasSpawnedProcessForTesting(extensionID: String) -> Bool {
        processes[extensionID] != nil
    }

    func spawnedHostPIDs() -> Set<pid_t> {
        Set(processes.values.map(\.processIdentifier))
    }

    static var defaultRootDirectory: URL {
        let home = FileManager.default.homeDirectoryForCurrentUser
        return home.appendingPathComponent(".config/muxy/extensions", isDirectory: true)
    }

    var rootDirectory: URL { rootDirectoryURL }

    func loadManifestsIfNeeded() {
        guard !hasLoadedFromDisk else { return }
        loadFromDisk()
    }

    func startAll() {
        loadManifestsIfNeeded()
        for index in statuses.indices where statuses[index].isEnabled {
            startExtension(at: index)
        }
        rebuildExtensionUICache()
        syncExtensionShortcuts()
        publishSnapshot()
        localizationsDidChange(self)
    }

    func stopAll() {
        for extensionID in Array(processes.keys) {
            stopProcess(extensionID: extensionID)
        }
        topbarOverrides.removeAll()
        statusBarOverrides.removeAll()
        ExtensionIconAssetCache.shared.invalidateAll()
        for status in statuses {
            ExtensionPanelRegistry.shared.closeAll(extensionID: status.id)
            PopoverHost.shared.close(extensionID: status.id)
            ExtensionWebviewModalService.shared.dismiss(extensionID: status.id)
        }
        hasLoadedFromDisk = false
        rebuildExtensionUICache()
        publishSnapshot()
    }

    func reload() {
        stopAll()
        startAll()
    }

    func addDevPath(_ path: String) {
        ExtensionDevPathStore.add(path)
        reload()
    }

    func removeDevPath(_ path: String) {
        ExtensionDevPathStore.remove(path)
        reload()
    }

    func install(expectedName: String, zip: Data) async throws {
        let staged = try await Task.detached {
            try Self.unpackAndValidate(expectedName: expectedName, zip: zip)
        }.value
        defer { try? FileManager.default.removeItem(at: staged.workspace) }

        let target = rootDirectoryURL.appendingPathComponent(expectedName, isDirectory: true)
        try FileManager.default.createDirectory(
            at: rootDirectoryURL,
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: FilePermissions.privateDirectory]
        )

        if statuses.contains(where: { $0.id == expectedName }) {
            stopProcess(extensionID: expectedName)
        }
        if FileManager.default.fileExists(atPath: target.path) {
            try FileManager.default.removeItem(at: target)
        }
        try FileManager.default.moveItem(at: staged.manifestRoot, to: target)

        reload()
    }

    func delete(extensionID: String) throws {
        guard let status = statuses.first(where: { $0.id == extensionID }) else { return }
        guard !status.isDev else { throw ExtensionDeleteError.devExtension }

        stopProcess(extensionID: extensionID)

        let target = rootDirectoryURL.appendingPathComponent(extensionID, isDirectory: true)
        if FileManager.default.fileExists(atPath: target.path) {
            try FileManager.default.removeItem(at: target)
        }

        ExtensionEnabledStore.clear(extensionID: extensionID)
        ExtensionSettingsStore.shared.clearAll(extensionID: extensionID)
        ExtensionGrantStore.shared.removeAll(for: extensionID)
        availableUpdates.removeValue(forKey: extensionID)

        reload()
    }

    func availableUpdateVersion(for extensionID: String) -> String? {
        availableUpdates[extensionID]
    }

    func checkForUpdates() async {
        let installed = statuses.filter { !$0.isDev }
        guard !installed.isEmpty else {
            availableUpdates = [:]
            return
        }
        let remote: [String: String]
        do {
            remote = try await marketplace.resolveVersions(names: installed.map(\.id))
        } catch {
            logger.error("Failed to check for extension updates: \(error.localizedDescription)")
            return
        }

        var updates: [String: String] = [:]
        for status in installed {
            guard let remoteVersion = remote[status.id] else { continue }
            let installedVersion = status.muxyExtension.manifest.version
            if SemanticVersion.isUpdate(installed: installedVersion, available: remoteVersion) {
                updates[status.id] = remoteVersion
            }
        }
        availableUpdates = updates
    }

    func update(extensionID: String) async throws {
        guard let status = statuses.first(where: { $0.id == extensionID }), !status.isDev else { return }
        let ext = try await marketplace.fetch(name: extensionID)
        let zip = try await marketplace.download(ext)
        try await install(expectedName: ext.name, zip: zip)
        availableUpdates.removeValue(forKey: extensionID)
    }

    struct UpdateAllResult {
        var succeeded: [String] = []
        var failed: [(id: String, message: String)] = []
    }

    func updateAll() async -> UpdateAllResult {
        var result = UpdateAllResult()
        for extensionID in availableUpdates.keys.sorted() {
            do {
                try await update(extensionID: extensionID)
                result.succeeded.append(extensionID)
            } catch {
                let message = (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
                result.failed.append((id: extensionID, message: message))
            }
        }
        return result
    }

    private struct StagedExtension {
        let manifestRoot: URL
        let workspace: URL
    }

    nonisolated private static func unpackAndValidate(expectedName: String, zip: Data) throws -> StagedExtension {
        let fileManager = FileManager.default
        let workspace = fileManager.temporaryDirectory
            .appendingPathComponent("muxy-ext-install-\(UUID().uuidString)", isDirectory: true)
        let extractDir = workspace.appendingPathComponent("extract", isDirectory: true)
        let archiveURL = workspace.appendingPathComponent("\(expectedName).zip")

        do {
            try fileManager.createDirectory(at: extractDir, withIntermediateDirectories: true)
            try zip.write(to: archiveURL)
            try runUnzip(archiveURL: archiveURL, destination: extractDir)

            let manifestRoot = try locateManifestRoot(in: extractDir)
            let loaded = try ExtensionManifestLoader.load(from: manifestRoot)
            guard loaded.id == expectedName else {
                throw MarketplaceError.invalidArchive
            }
            return StagedExtension(manifestRoot: manifestRoot, workspace: workspace)
        } catch {
            try? fileManager.removeItem(at: workspace)
            if error is MarketplaceError {
                throw error
            }
            if error is ExtensionLoadError {
                throw MarketplaceError.invalidArchive
            }
            throw MarketplaceError.unpackFailed(error.localizedDescription)
        }
    }

    nonisolated private static func runUnzip(archiveURL: URL, destination: URL) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/unzip")
        process.arguments = ["-o", "-q", archiveURL.path, "-d", destination.path]
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            throw MarketplaceError.unpackFailed("unzip exited with status \(process.terminationStatus)")
        }
    }

    nonisolated private static func locateManifestRoot(in directory: URL) throws -> URL {
        let fileManager = FileManager.default
        let manifestFile = ExtensionManifestLoader.manifestFileName
        if fileManager.fileExists(atPath: directory.appendingPathComponent(manifestFile).path) {
            return directory
        }
        let entries = (try? fileManager.contentsOfDirectory(
            at: directory,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles]
        )) ?? []
        let directories = entries.filter { url in
            (try? url.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true
        }
        guard directories.count == 1,
              fileManager.fileExists(atPath: directories[0].appendingPathComponent(manifestFile).path)
        else {
            throw MarketplaceError.invalidArchive
        }
        return directories[0]
    }

    func setEnabled(_ enabled: Bool, for extensionID: String) {
        guard let index = statuses.firstIndex(where: { $0.id == extensionID }) else { return }
        ExtensionEnabledStore.setEnabled(enabled, extensionID: extensionID)
        statuses[index].isEnabled = enabled

        if enabled, !statuses[index].isRunning {
            crashRestartAttempts.removeValue(forKey: extensionID)
            startExtension(at: index)
        } else if !enabled, statuses[index].isRunning {
            stopProcess(extensionID: extensionID)
        }
        if !enabled {
            topbarOverrides.removeValue(forKey: extensionID)
            statusBarOverrides.removeValue(forKey: extensionID)
            ExtensionIconAssetCache.shared.invalidate(extensionID: extensionID)
            ExtensionPanelRegistry.shared.closeAll(extensionID: extensionID)
            PopoverHost.shared.close(extensionID: extensionID)
            ExtensionWebviewModalService.shared.dismiss(extensionID: extensionID)
        }
        rebuildExtensionUICache()
        syncExtensionShortcuts()
        publishSnapshot()
        localizationsDidChange(self)
    }

    func extensionHasPermission(id: String, permission: ExtensionPermission) -> Bool {
        guard let muxyExtension = loadedExtension(id: id) else { return false }
        return muxyExtension.manifest.permissions.contains(permission)
    }

    func loadedExtension(id: String) -> MuxyExtension? {
        statuses.first(where: { $0.id == id && $0.isEnabled })?.muxyExtension
    }

    func snapshotForSocketServer() -> NotificationSocketServer.ExtensionSnapshot {
        var entries: [String: NotificationSocketServer.ExtensionSnapshotEntry] = [:]
        for status in statuses where status.isEnabled {
            let manifest = status.muxyExtension.manifest
            guard let token = tokens[status.id] else { continue }
            let runtimeCommandEvents = ExtensionShortcutStore.shared.runtimeShortcuts
                .filter { $0.extensionID == status.id }
                .map(\.eventName)
            entries[status.id] = NotificationSocketServer.ExtensionSnapshotEntry(
                allowedEvents: Set(manifest.events),
                commandEvents: Set(manifest.commands.map(\.eventName)).union(runtimeCommandEvents),
                permissions: Set(manifest.permissions),
                token: token
            )
        }
        return NotificationSocketServer.ExtensionSnapshot(entries: entries)
    }

    func refreshExtensionSnapshot() {
        publishSnapshotSync()
    }

    private func publishSnapshot() {
        snapshotSink.applyExtensionSnapshot(snapshotForSocketServer())
    }

    private func publishSnapshotSync() {
        snapshotSink.applyExtensionSnapshotSync(snapshotForSocketServer())
    }

    static func buildSnapshotForTesting(
        from entries: [(MuxyExtension, isEnabled: Bool)],
        token: String = "test-token"
    ) -> NotificationSocketServer.ExtensionSnapshot {
        var result: [String: NotificationSocketServer.ExtensionSnapshotEntry] = [:]
        for (ext, isEnabled) in entries where isEnabled {
            let manifest = ext.manifest
            result[ext.id] = NotificationSocketServer.ExtensionSnapshotEntry(
                allowedEvents: Set(manifest.events),
                commandEvents: Set(manifest.commands.map(\.eventName)),
                permissions: Set(manifest.permissions),
                token: token
            )
        }
        return NotificationSocketServer.ExtensionSnapshot(entries: result)
    }

    struct PaletteCommandBinding: Equatable {
        let muxyExtension: MuxyExtension
        let command: ExtensionPaletteCommand
    }

    struct FileOpenerBinding: Equatable, Identifiable {
        let muxyExtension: MuxyExtension
        let opener: ExtensionFileOpener
        let tabType: ExtensionTabType

        var id: String {
            "\(muxyExtension.id):\(opener.id)"
        }
    }

    struct LocalizationBinding: Equatable, Identifiable {
        let muxyExtension: MuxyExtension
        let localization: ExtensionLocalization

        var id: String {
            LocalizationSelection.value(
                extensionID: muxyExtension.id,
                localizationID: localization.id
            )
        }

        var bundleURL: URL? {
            muxyExtension.resolveResource(localization.bundle)
        }
    }

    struct ItemOverride: Equatable {
        var icon: ExtensionIcon?
        var text: String?
        var visible: Bool?
    }

    struct TopbarItemBinding: Equatable, Identifiable {
        let muxyExtension: MuxyExtension
        let item: ExtensionTopbarItem
        let liveIcon: ExtensionIcon?
        let liveVisible: Bool?

        var id: String { "\(muxyExtension.id):\(item.id)" }
        var displayIcon: ExtensionIcon { liveIcon ?? item.icon }
        var isVisible: Bool { liveVisible ?? item.visible }
        var isRailEligible: Bool {
            guard let command = muxyExtension.manifest.commands.first(where: { $0.id == item.command })
            else { return false }
            return command.action.isRailEligible
        }
    }

    struct StatusBarItemBinding: Equatable, Identifiable {
        let muxyExtension: MuxyExtension
        let item: ExtensionStatusBarItem
        let liveIcon: ExtensionIcon?
        let liveText: String?
        let liveVisible: Bool?

        var id: String { "\(muxyExtension.id):\(item.id)" }
        var displayIcon: ExtensionIcon { liveIcon ?? item.icon }
        var displayText: String? { liveText ?? item.text }
        var isVisible: Bool { liveVisible ?? item.visible }
    }

    private var topbarOverrides: [String: [String: ItemOverride]] = [:]
    private var statusBarOverrides: [String: [String: ItemOverride]] = [:]
    private(set) var topbarItems: [TopbarItemBinding] = []
    private(set) var leftStatusBarItems: [StatusBarItemBinding] = []
    private(set) var rightStatusBarItems: [StatusBarItemBinding] = []

    func paletteCommands() -> [PaletteCommandBinding] {
        statuses
            .filter(\.isEnabled)
            .flatMap { status in
                status.muxyExtension.manifest.commands
                    .filter { !$0.action.isAnchored }
                    .map { PaletteCommandBinding(muxyExtension: status.muxyExtension, command: $0) }
            }
    }

    func fileOpeners(for relativePath: String) -> [FileOpenerBinding] {
        fileOpeners().filter { $0.opener.matches(relativePath: relativePath) }
    }

    func fileOpeners() -> [FileOpenerBinding] {
        statuses
            .filter(\.isEnabled)
            .flatMap { status in
                let ext = status.muxyExtension
                return ext.manifest.fileOpeners.compactMap { opener -> FileOpenerBinding? in
                    guard let tabType = ext.manifest.tabType(id: opener.tabType)
                    else {
                        return nil
                    }
                    return FileOpenerBinding(muxyExtension: ext, opener: opener, tabType: tabType)
                }
            }
    }

    func fileOpener(extensionID: String, openerID: String, relativePath: String? = nil) -> FileOpenerBinding? {
        fileOpeners().first { binding in
            let matchesPath = relativePath.map { binding.opener.matches(relativePath: $0) } ?? true
            return binding.muxyExtension.id == extensionID &&
                binding.opener.id == openerID &&
                matchesPath
        }
    }

    func localizations() -> [LocalizationBinding] {
        statuses
            .filter(\.isEnabled)
            .flatMap { status in
                let ext = status.muxyExtension
                return ext.manifest.localizations.map {
                    LocalizationBinding(muxyExtension: ext, localization: $0)
                }
            }
    }

    func localization(extensionID: String, localizationID: String) -> LocalizationBinding? {
        localizations().first {
            $0.muxyExtension.id == extensionID && $0.localization.id == localizationID
        }
    }

    func preferredFileOpener(for relativePath: String) -> FileOpenerBinding? {
        FileOpenerSelection.resolvedBinding(
            from: IDEIntegrationService.shared.selectedFileOpenerValue,
            relativePath: relativePath,
            store: self
        )
    }

    func statusBarItems(side: ExtensionStatusBarItem.Side) -> [StatusBarItemBinding] {
        side == .left ? leftStatusBarItems : rightStatusBarItems
    }

    func popover(for muxyExtension: MuxyExtension, command commandID: String) -> ExtensionPopover? {
        guard let command = muxyExtension.manifest.commands.first(where: { $0.id == commandID }),
              case let .openPopover(popoverID) = command.action
        else { return nil }
        guard commandCanRun(command, extensionID: muxyExtension.id, logFailure: false) else { return nil }
        return muxyExtension.manifest.popover(id: popoverID)
    }

    private func rebuildExtensionUICache() {
        var topbar: [TopbarItemBinding] = []
        var left: [StatusBarItemBinding] = []
        var right: [StatusBarItemBinding] = []
        for status in statuses where status.isEnabled {
            let ext = status.muxyExtension
            let topbarItemOverrides = topbarOverrides[status.id]
            let statusBarItemOverrides = statusBarOverrides[status.id]
            for item in ext.manifest.topbarItems {
                let override = topbarItemOverrides?[item.id]
                let binding = TopbarItemBinding(
                    muxyExtension: ext,
                    item: item,
                    liveIcon: override?.icon,
                    liveVisible: override?.visible
                )
                guard binding.isVisible else { continue }
                topbar.append(binding)
            }
            for item in ext.manifest.statusBarItems {
                let override = statusBarItemOverrides?[item.id]
                let binding = StatusBarItemBinding(
                    muxyExtension: ext,
                    item: item,
                    liveIcon: override?.icon,
                    liveText: override?.text,
                    liveVisible: override?.visible
                )
                guard binding.isVisible else { continue }
                switch item.side {
                case .left: left.append(binding)
                case .right: right.append(binding)
                }
            }
        }
        topbarItems = topbar
        leftStatusBarItems = left
        rightStatusBarItems = right
    }

    private func syncExtensionShortcuts() {
        let enabled = statuses.filter(\.isEnabled).map(\.muxyExtension)
        ExtensionShortcutStore.shared.syncBindings(for: enabled)
        ExtensionShortcutStore.shared.clearRuntimeShortcuts(keepingExtensionIDs: Set(enabled.map(\.id)))
    }

    func triggerRuntimeShortcut(extensionID: String, commandID: String) {
        broadcastCommandEvent(
            extensionID: extensionID,
            commandID: commandID,
            name: ExtensionShortcut.eventName(forCommandID: commandID)
        )
    }

    struct StatusBarUpdate {
        var icon: ExtensionIcon?
        var text: String?
        var clearText: Bool = false
        var visible: Bool?
    }

    func setStatusBarText(extensionID: String, itemID: String, text: String?) -> Bool {
        setStatusBarItem(extensionID: extensionID, itemID: itemID, update: StatusBarUpdate(text: text, clearText: true))
    }

    func setStatusBarItem(extensionID: String, itemID: String, update: StatusBarUpdate) -> Bool {
        guard loadedExtension(id: extensionID)?.manifest.statusBarItem(id: itemID) != nil
        else { return false }

        applyOverride(to: \.statusBarOverrides, extensionID: extensionID, itemID: itemID) {
            if let icon = update.icon {
                $0.icon = icon
            }
            if update.clearText {
                $0.text = update.text
            }
            if let visible = update.visible {
                $0.visible = visible
            }
        }
        return true
    }

    func setTopbarItem(extensionID: String, itemID: String, icon: ExtensionIcon?, visible: Bool?) -> Bool {
        guard loadedExtension(id: extensionID)?.manifest.topbarItem(id: itemID) != nil
        else { return false }

        applyOverride(to: \.topbarOverrides, extensionID: extensionID, itemID: itemID) {
            if let icon {
                $0.icon = icon
            }
            if let visible {
                $0.visible = visible
            }
        }
        return true
    }

    private func applyOverride(
        to keyPath: ReferenceWritableKeyPath<ExtensionStore, [String: [String: ItemOverride]]>,
        extensionID: String,
        itemID: String,
        mutate: (inout ItemOverride) -> Void
    ) {
        var overrides = self[keyPath: keyPath][extensionID] ?? [:]
        var override = overrides[itemID] ?? ItemOverride()
        mutate(&override)
        if override.icon == nil, override.text == nil, override.visible == nil {
            overrides.removeValue(forKey: itemID)
        } else {
            overrides[itemID] = override
        }
        self[keyPath: keyPath][extensionID] = overrides.isEmpty ? nil : overrides
        rebuildExtensionUICache()
    }

    struct CommandInvocation {
        let extensionID: String
        let commandID: String
        let appState: AppState
        let projectStore: ProjectStore?
        let worktreeStore: WorktreeStore?
        let projectGroupStore: ProjectGroupStore?
        let browserProfileStore: BrowserProfileStore?

        init(
            extensionID: String,
            commandID: String,
            appState: AppState,
            projectStore: ProjectStore? = nil,
            worktreeStore: WorktreeStore? = nil,
            projectGroupStore: ProjectGroupStore? = nil,
            browserProfileStore: BrowserProfileStore? = nil
        ) {
            self.extensionID = extensionID
            self.commandID = commandID
            self.appState = appState
            self.projectStore = projectStore
            self.worktreeStore = worktreeStore
            self.projectGroupStore = projectGroupStore
            self.browserProfileStore = browserProfileStore
        }
    }

    func triggerCommand(_ invocation: CommandInvocation) {
        guard let muxyExtension = loadedExtension(id: invocation.extensionID),
              let command = muxyExtension.manifest.commands.first(where: { $0.id == invocation.commandID })
        else { return }
        guard commandCanRun(command, extensionID: invocation.extensionID) else { return }

        switch command.action {
        case .event:
            broadcastCommandEvent(
                extensionID: invocation.extensionID,
                commandID: invocation.commandID,
                name: command.eventName
            )
        case let .openTab(tabType, data):
            openExtensionTab(
                extensionID: invocation.extensionID,
                tabType: tabType,
                data: data,
                in: muxyExtension,
                appState: invocation.appState
            )
        case let .togglePanel(panelID):
            guard let panel = muxyExtension.manifest.panel(id: panelID) else { return }
            ExtensionPanelRegistry.shared.toggle(
                extensionID: invocation.extensionID,
                panel: panel,
                data: nil
            )
        case .openPopover:
            break
        case let .openModal(modal):
            ExtensionWebviewModalService.shared.open(
                ExtensionWebviewModalService.OpenRequest(
                    extensionID: invocation.extensionID,
                    entry: modal.entry,
                    width: modal.width,
                    height: modal.height,
                    dismissOnOutsideClick: modal.dismissOnOutsideClick,
                    data: modal.data
                )
            )
        case let .runScript(script):
            runExtensionScript(script: script, in: muxyExtension, invocation: invocation)
        }
    }

    private func commandCanRun(
        _ command: ExtensionPaletteCommand,
        extensionID: String,
        logFailure: Bool = true
    ) -> Bool {
        guard let permission = command.action.requiredPermission else { return true }
        guard extensionHasPermission(id: extensionID, permission: permission) else {
            if logFailure {
                ExtensionLogStore.shared.append(
                    extensionID: extensionID,
                    line: "[muxy] command \(command.id) blocked: missing \(permission.rawValue) permission"
                )
            }
            return false
        }
        return true
    }

    private func runExtensionScript(
        script: String,
        in muxyExtension: MuxyExtension,
        invocation: CommandInvocation
    ) {
        guard let scriptURL = muxyExtension.resolveResource(script) else {
            ExtensionLogStore.shared.append(
                extensionID: invocation.extensionID,
                line: "[muxy] runScript blocked: script path escapes extension directory"
            )
            return
        }
        Task { @MainActor in
            do {
                try await ExtensionScriptRunner.shared.runScript(
                    extensionID: invocation.extensionID,
                    scriptURL: scriptURL,
                    appState: invocation.appState,
                    stores: ExtensionAPIStores(
                        projectStore: invocation.projectStore,
                        worktreeStore: invocation.worktreeStore,
                        projectGroupStore: invocation.projectGroupStore,
                        browserProfileStore: invocation.browserProfileStore
                    )
                )
            } catch {
                ExtensionLogStore.shared.append(
                    extensionID: invocation.extensionID,
                    line: "[muxy] runScript failed: \(error.localizedDescription)"
                )
            }
        }
    }

    private func broadcastCommandEvent(extensionID: String, commandID: String, name: String) {
        NotificationSocketServer.shared.broadcast(
            event: ExtensionEvent(
                name: name,
                payload: ["extension": extensionID, "command": commandID]
            )
        )
    }

    private func openExtensionTab(
        extensionID: String,
        tabType tabTypeID: String,
        data: ExtensionJSON?,
        in muxyExtension: MuxyExtension,
        appState: AppState
    ) {
        guard let tabType = muxyExtension.manifest.tabType(id: tabTypeID),
              let projectID = appState.activeProjectID
        else { return }
        appState.dispatch(.createExtensionTab(
            projectID: projectID,
            areaID: nil,
            request: AppState.CreateExtensionTabRequest(
                extensionID: extensionID,
                tabTypeID: tabTypeID,
                title: tabType.title,
                data: data ?? tabType.defaultData,
                singleton: false
            )
        ))
    }

    private func loadFromDisk() {
        statuses = []
        loadFailures = []

        try? FileManager.default.createDirectory(
            at: rootDirectoryURL,
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: FilePermissions.privateDirectory]
        )

        guard let contents = try? FileManager.default.contentsOfDirectory(
            at: rootDirectoryURL,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles]
        )
        else { return }

        var seenIDs = Set<String>()
        for url in contents.sorted(by: { $0.lastPathComponent < $1.lastPathComponent }) {
            var isDirectory: ObjCBool = false
            guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory),
                  isDirectory.boolValue
            else { continue }
            loadOne(at: url, enforceNameMatchesDirectory: true, seenIDs: &seenIDs)
        }

        for path in devPathsProvider() {
            loadOne(
                at: URL(fileURLWithPath: path, isDirectory: true),
                enforceNameMatchesDirectory: false,
                devSourcePath: path,
                seenIDs: &seenIDs
            )
        }
        pruneResolvedUpdates()
        hasLoadedFromDisk = true
        localizationsDidChange(self)
    }

    private func loadOne(
        at url: URL,
        enforceNameMatchesDirectory: Bool,
        devSourcePath: String? = nil,
        seenIDs: inout Set<String>
    ) {
        do {
            let ext = try ExtensionManifestLoader.load(from: url)
            if enforceNameMatchesDirectory, ext.id != url.lastPathComponent {
                throw ExtensionLoadError.nameDirectoryMismatch(
                    name: ext.id,
                    directory: url.lastPathComponent
                )
            }
            guard !seenIDs.contains(ext.id) else {
                loadFailures.append(LoadFailure(
                    directory: url,
                    message: ExtensionLoadError.duplicateName(ext.id).localizedDescription,
                    devSourcePath: devSourcePath
                ))
                return
            }
            seenIDs.insert(ext.id)
            ExtensionLogStore.shared.register(extensionID: ext.id, directory: ext.directory)
            statuses.append(ExtensionStatus(
                id: ext.id,
                muxyExtension: ext,
                isEnabled: ExtensionEnabledStore.isEnabled(extensionID: ext.id),
                isRunning: false,
                isDev: devSourcePath != nil,
                devSourcePath: devSourcePath,
                lastError: nil
            ))
        } catch {
            loadFailures.append(LoadFailure(
                directory: url,
                message: error.localizedDescription,
                devSourcePath: devSourcePath
            ))
            logger.error("Failed to load extension at \(url.path): \(error.localizedDescription)")
        }
    }

    private func pruneResolvedUpdates() {
        guard !availableUpdates.isEmpty else { return }
        availableUpdates = availableUpdates.filter { id, remoteVersion in
            guard let installed = statuses.first(where: { $0.id == id })?.muxyExtension.manifest.version
            else { return false }
            return SemanticVersion.isUpdate(installed: installed, available: remoteVersion)
        }
    }

    private func startExtension(at index: Int) {
        let status = statuses[index]
        let ext = status.muxyExtension

        guard let backgroundScriptURL = ext.backgroundScriptURL else { return }

        guard let hostURL = resolveHostURL() else {
            let message = "Extension host binary not found"
            statuses[index].lastError = message
            ExtensionLogStore.shared.append(extensionID: ext.id, line: "[muxy] \(message)")
            logger.error("Cannot start extension \(ext.id): \(message)")
            return
        }

        let process = Process()
        process.executableURL = hostURL
        process.arguments = [backgroundScriptURL.path]
        process.currentDirectoryURL = ext.directory

        let token = Self.generateToken()
        tokens[ext.id] = token
        publishSnapshotSync()

        var environment = ProcessInfo.processInfo.environment
        environment["MUXY_SOCKET_PATH"] = NotificationSocketServer.socketPath
        environment["MUXY_EXTENSION_ID"] = ext.id
        environment["MUXY_EXTENSION_TOKEN"] = token
        process.environment = environment

        let logURL = ExtensionLogStore.shared.logURL(extensionID: ext.id, directory: ext.directory)

        if let logHandle = openProcessLogHandle(at: logURL) {
            process.standardOutput = logHandle
            process.standardError = logHandle
        }

        process.terminationHandler = { [weak self] terminatedProcess in
            Task { @MainActor [weak self] in
                self?.handleTermination(extensionID: ext.id, process: terminatedProcess)
            }
        }

        do {
            try process.run()
            processes[ext.id] = process
            let processGroupID = getpgid(process.processIdentifier)
            if processGroupID > 0 {
                hostPGIDs[ext.id] = processGroupID
            }
            statuses[index].isRunning = true
            statuses[index].lastError = nil
            scheduleCrashCounterReset(extensionID: ext.id, process: process)
            ExtensionLogStore.shared.append(
                extensionID: ext.id,
                line: "[muxy] started \(ext.id) v\(ext.manifest.version)"
            )
        } catch {
            tokens.removeValue(forKey: ext.id)
            publishSnapshot()
            statuses[index].lastError = error.localizedDescription
            ExtensionLogStore.shared.append(
                extensionID: ext.id,
                line: "[muxy] failed to start: \(error.localizedDescription)"
            )
            logger.error("Failed to start extension \(ext.id): \(error.localizedDescription)")
        }
    }

    private func openProcessLogHandle(at url: URL) -> FileHandle? {
        let directory = url.deletingLastPathComponent()
        if !FileManager.default.fileExists(atPath: directory.path) {
            try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        }
        if !FileManager.default.fileExists(atPath: url.path) {
            FileManager.default.createFile(atPath: url.path, contents: nil)
        }
        guard let handle = try? FileHandle(forWritingTo: url) else { return nil }
        _ = try? handle.seekToEnd()
        return handle
    }

    private func stopProcess(extensionID: String) {
        crashRestartTasks.removeValue(forKey: extensionID)?.cancel()
        crashRestartAttempts.removeValue(forKey: extensionID)
        ExtensionScriptRunner.shared.evict(extensionID: extensionID)
        tokens.removeValue(forKey: extensionID)
        hostPGIDs.removeValue(forKey: extensionID)
        guard let process = processes.removeValue(forKey: extensionID) else { return }
        if process.isRunning {
            intentionalStops.insert(extensionID)
            Self.terminateProcessTree(pid: process.processIdentifier)
        }
        if let index = statuses.firstIndex(where: { $0.id == extensionID }) {
            statuses[index].isRunning = false
        }
    }

    nonisolated private static func terminateProcessTree(pid: pid_t) {
        let group = getpgid(pid)
        let target = group > 0 ? group : pid
        killpg(target, SIGTERM)
        DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + processTerminationGracePeriod) {
            killpg(target, SIGKILL)
        }
    }

    nonisolated static func terminateProcessTreeForTesting(pid: pid_t) {
        terminateProcessTree(pid: pid)
    }

    private static func generateToken() -> String {
        var bytes = [UInt8](repeating: 0, count: 32)
        let result = SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes)
        guard result == errSecSuccess else {
            return UUID().uuidString + UUID().uuidString
        }
        return Data(bytes).base64EncodedString()
    }

    private func handleTermination(extensionID: String, process: Process) {
        let wasIntentional = intentionalStops.remove(extensionID) != nil
        guard processes[extensionID] === process else { return }
        processes.removeValue(forKey: extensionID)
        guard let index = statuses.firstIndex(where: { $0.id == extensionID }) else { return }
        statuses[index].isRunning = false
        PopoverHost.shared.close(extensionID: extensionID)
        ExtensionWebviewModalService.shared.dismiss(extensionID: extensionID)
        let outcome = Self.classifyTermination(
            wasIntentional: wasIntentional,
            terminationStatus: process.terminationStatus
        )
        switch outcome {
        case .stopped:
            ExtensionLogStore.shared.append(extensionID: extensionID, line: "[muxy] stopped")
        case .exitedCleanly:
            ExtensionLogStore.shared.append(extensionID: extensionID, line: "[muxy] exited cleanly")
        case let .exitedWithStatus(status):
            let message = "Process exited with status \(status)"
            statuses[index].lastError = message
            ExtensionLogStore.shared.append(extensionID: extensionID, line: "[muxy] \(message)")
            scheduleCrashRestart(extensionID: extensionID)
        }
    }

    nonisolated static func shouldRestartAfterCrash(isEnabled: Bool, attempts: Int) -> Bool {
        isEnabled && attempts < maxCrashRestartAttempts
    }

    private func scheduleCrashRestart(extensionID: String) {
        guard let index = statuses.firstIndex(where: { $0.id == extensionID }) else { return }
        let attempts = crashRestartAttempts[extensionID, default: 0]
        guard Self.shouldRestartAfterCrash(isEnabled: statuses[index].isEnabled, attempts: attempts) else {
            guard statuses[index].isEnabled else { return }
            let message = "Stopped after \(Self.maxCrashRestartAttempts) crash restarts"
            statuses[index].lastError = message
            ExtensionLogStore.shared.append(extensionID: extensionID, line: "[muxy] \(message)")
            return
        }
        let nextAttempt = attempts + 1
        crashRestartAttempts[extensionID] = nextAttempt
        ExtensionLogStore.shared.append(
            extensionID: extensionID,
            line: "[muxy] restarting after crash (attempt \(nextAttempt)/\(Self.maxCrashRestartAttempts))"
        )
        crashRestartTasks[extensionID] = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(Self.crashRestartBackoff))
            guard !Task.isCancelled, let self else { return }
            self.crashRestartTasks.removeValue(forKey: extensionID)
            guard let index = self.statuses.firstIndex(where: { $0.id == extensionID }),
                  self.statuses[index].isEnabled, !self.statuses[index].isRunning
            else { return }
            self.startExtension(at: index)
        }
    }

    private func scheduleCrashCounterReset(extensionID: String, process: Process) {
        guard crashRestartAttempts[extensionID] != nil else { return }
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(Self.crashStabilityWindow))
            guard let self, self.processes[extensionID] === process else { return }
            self.crashRestartAttempts.removeValue(forKey: extensionID)
        }
    }

    enum TerminationOutcome: Equatable {
        case stopped
        case exitedCleanly
        case exitedWithStatus(Int32)
    }

    nonisolated static func classifyTermination(wasIntentional: Bool, terminationStatus: Int32) -> TerminationOutcome {
        if wasIntentional {
            return .stopped
        }
        if terminationStatus == 0 {
            return .exitedCleanly
        }
        return .exitedWithStatus(terminationStatus)
    }
}

enum ExtensionDeleteError: LocalizedError, Equatable {
    case devExtension

    var errorDescription: String? {
        switch self {
        case .devExtension:
            "Unpacked extensions can only be removed with “Remove from Muxy”."
        }
    }
}
