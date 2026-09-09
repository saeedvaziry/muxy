import Foundation
import os

private let logger = Logger(subsystem: "app.muxy", category: "BackupService")

private actor BackupOperationLock {
    private var isLocked = false
    private var waiters: [CheckedContinuation<Void, Never>] = []

    func withLock<T: Sendable>(_ operation: @MainActor @Sendable () async throws -> T) async rethrows -> T {
        await acquire()
        defer { release() }
        return try await operation()
    }

    private func acquire() async {
        guard isLocked else {
            isLocked = true
            return
        }
        await withCheckedContinuation { waiters.append($0) }
    }

    private func release() {
        guard !waiters.isEmpty else {
            isLocked = false
            return
        }
        waiters.removeFirst().resume()
    }
}

struct BackupService {
    typealias SettingsSynchronizer = @MainActor @Sendable () -> SettingsJSONStore.SyncResult
    typealias SettingsApplier = @MainActor @Sendable (URL) throws -> Void

    private static let operationLock = BackupOperationLock()

    let baseDirectory: URL
    private let settingsSynchronizer: SettingsSynchronizer
    private let settingsApplier: SettingsApplier

    init(
        baseDirectory: URL = MuxyFileStorage.appSupportDirectory(),
        settingsSynchronizer: @escaping SettingsSynchronizer = {
            SettingsJSONStore.syncUserSettingsFileWithCurrentSettings()
        },
        settingsApplier: @escaping SettingsApplier = {
            try SettingsJSONStore.applyUserSettingsFile(from: $0)
        }
    ) {
        self.baseDirectory = baseDirectory
        self.settingsSynchronizer = settingsSynchronizer
        self.settingsApplier = settingsApplier
    }

    func export(to archiveURL: URL, appVersion: String, createdAt: Date) async throws {
        try await GitProcessRunner.offMainThrowing {
            try performExport(to: archiveURL, appVersion: appVersion, createdAt: createdAt)
        }
    }

    private func performExport(to archiveURL: URL, appVersion: String, createdAt: Date) throws {
        let staging = try makeTemporaryDirectory()
        defer { try? FileManager.default.removeItem(at: staging) }

        let copiedFiles = try stageExportContents(into: staging)
        let manifest = BackupManifest(
            schemaVersion: BackupManifest.currentSchemaVersion,
            appVersion: appVersion,
            createdAt: createdAt,
            files: copiedFiles
        )
        try writeManifest(manifest, into: staging)

        try? FileManager.default.removeItem(at: archiveURL)
        try BackupArchive.zip(directory: staging, to: archiveURL)
        try? FileManager.default.setAttributes(
            [.posixPermissions: FilePermissions.privateFile],
            ofItemAtPath: archiveURL.path
        )
    }

    @discardableResult
    func importBackup(from archiveURL: URL, backupStamp: String) async throws -> URL {
        try await GitProcessRunner.offMainThrowing {
            try performImport(from: archiveURL, backupStamp: backupStamp)
        }
    }

    @MainActor
    func exportCurrent(to archiveURL: URL, createdAt: Date = Date()) async throws {
        try await BackupService.operationLock.withLock {
            switch settingsSynchronizer() {
            case .updated,
                 .unchanged:
                break
            case .failed:
                throw SettingsJSONError.syncFailed
            }
            try await export(to: archiveURL, appVersion: currentAppVersion, createdAt: createdAt)
        }
    }

    @MainActor
    func importAndApply(from archiveURL: URL) async throws {
        try await BackupService.operationLock.withLock {
            let backupDirectory = try await importBackup(from: archiveURL, backupStamp: backupStamp())
            do {
                try settingsApplier(baseDirectory.appendingPathComponent(SettingsCatalog.userSettingsFilename))
            } catch {
                try await restoreFromPreImport(backupDirectory: backupDirectory)
                throw error
            }
        }
    }

    private func restoreFromPreImport(backupDirectory: URL) async throws {
        try await GitProcessRunner.offMainThrowing {
            try restoreCurrentData(from: backupDirectory)
        }
    }

    @discardableResult
    private func performImport(from archiveURL: URL, backupStamp: String) throws -> URL {
        let staging = try makeTemporaryDirectory()
        defer { try? FileManager.default.removeItem(at: staging) }

        try BackupArchive.unzip(archiveURL: archiveURL, to: staging)
        let manifest = try readManifest(from: staging)
        guard manifest.isSupported else { throw BackupArchiveError.manifestUnsupported }

        let backupDirectory = try createCurrentDataBackupAndClearActiveData(stamp: backupStamp)
        do {
            try restoreContents(from: staging, manifest: manifest)
        } catch {
            try? restoreCurrentData(from: backupDirectory)
            throw error
        }
        return backupDirectory
    }

    private func stageExportContents(into staging: URL) throws -> [String] {
        let fileManager = FileManager.default
        var copied: [String] = []

        for name in BackupArchive.exportableFiles {
            let source = baseDirectory.appendingPathComponent(name)
            guard fileManager.fileExists(atPath: source.path) else { continue }
            let destination = staging.appendingPathComponent(name)

            if let sanitized = try sanitizedData(for: name, at: source) {
                try sanitized.write(to: destination, options: .atomic)
            } else {
                try fileManager.copyItem(at: source, to: destination)
            }
            copied.append(name)
        }

        for name in BackupArchive.exportableDirectories {
            let source = baseDirectory.appendingPathComponent(name, isDirectory: true)
            var isDirectory: ObjCBool = false
            guard fileManager.fileExists(atPath: source.path, isDirectory: &isDirectory), isDirectory.boolValue else { continue }
            try fileManager.copyItem(at: source, to: staging.appendingPathComponent(name, isDirectory: true))
            copied.append(name)
        }

        return copied
    }

    private func sanitizedData(for name: String, at url: URL) throws -> Data? {
        switch name {
        case "remote-devices.json":
            try BackupSanitizer.sanitizedRemoteDevices(at: url)
        case "settings.json":
            try BackupSanitizer.sanitizedSettings(at: url)
        default:
            nil
        }
    }

    private func createCurrentDataBackupAndClearActiveData(stamp: String) throws -> URL {
        let backupDirectory = try makePreImportBackupDirectory(stamp: stamp)

        do {
            try copyCurrentData(to: backupDirectory)
        } catch {
            try? FileManager.default.removeItem(at: backupDirectory)
            throw error
        }

        do {
            try removeCurrentData()
        } catch {
            try? restoreCurrentData(from: backupDirectory)
            throw error
        }

        return backupDirectory
    }

    private func makePreImportBackupDirectory(stamp: String) throws -> URL {
        let fileManager = FileManager.default
        let backupsDirectory = baseDirectory.appendingPathComponent("Backups", isDirectory: true)
        try fileManager.createDirectory(at: backupsDirectory, withIntermediateDirectories: true)

        var backupDirectory = backupsDirectory.appendingPathComponent("pre-import-\(stamp)", isDirectory: true)
        var suffix = 1
        while fileManager.fileExists(atPath: backupDirectory.path) {
            backupDirectory = backupsDirectory.appendingPathComponent("pre-import-\(stamp)-\(suffix)", isDirectory: true)
            suffix += 1
        }
        try fileManager.createDirectory(at: backupDirectory, withIntermediateDirectories: true)
        return backupDirectory
    }

    private func copyCurrentData(to backupDirectory: URL) throws {
        let fileManager = FileManager.default
        for name in BackupArchive.exportableFiles + BackupArchive.exportableDirectories {
            let source = baseDirectory.appendingPathComponent(name)
            guard fileManager.fileExists(atPath: source.path) else { continue }
            try fileManager.copyItem(at: source, to: backupDirectory.appendingPathComponent(name))
        }
    }

    private func removeCurrentData() throws {
        let fileManager = FileManager.default
        for name in BackupArchive.exportableFiles + BackupArchive.exportableDirectories {
            let url = baseDirectory.appendingPathComponent(name)
            guard fileManager.fileExists(atPath: url.path) else { continue }
            try fileManager.removeItem(at: url)
        }
    }

    private func restoreCurrentData(from backupDirectory: URL) throws {
        let fileManager = FileManager.default

        for name in BackupArchive.exportableFiles + BackupArchive.exportableDirectories {
            let destination = baseDirectory.appendingPathComponent(name)
            if fileManager.fileExists(atPath: destination.path) {
                try fileManager.removeItem(at: destination)
            }
            let source = backupDirectory.appendingPathComponent(name)
            guard fileManager.fileExists(atPath: source.path) else { continue }
            try fileManager.copyItem(at: source, to: destination)
        }
    }

    private func restoreContents(from staging: URL, manifest: BackupManifest) throws {
        let fileManager = FileManager.default
        let allowed = Set(BackupArchive.exportableFiles + BackupArchive.exportableDirectories)

        for name in manifest.files {
            guard allowed.contains(name) else {
                logger.error("Skipping unexpected backup entry: \(name)")
                continue
            }
            let source = staging.appendingPathComponent(name)
            guard fileManager.fileExists(atPath: source.path) else { continue }
            let destination = baseDirectory.appendingPathComponent(name)
            if fileManager.fileExists(atPath: destination.path) {
                try fileManager.removeItem(at: destination)
            }
            try fileManager.copyItem(at: source, to: destination)
        }
    }

    private func writeManifest(_ manifest: BackupManifest, into directory: URL) throws {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        encoder.dateEncodingStrategy = .iso8601
        let data = try encoder.encode(manifest)
        try data.write(to: directory.appendingPathComponent(BackupManifest.filename), options: .atomic)
    }

    private func readManifest(from directory: URL) throws -> BackupManifest {
        let url = directory.appendingPathComponent(BackupManifest.filename)
        guard FileManager.default.fileExists(atPath: url.path) else { throw BackupArchiveError.manifestMissing }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        do {
            return try decoder.decode(BackupManifest.self, from: Data(contentsOf: url))
        } catch {
            throw BackupArchiveError.manifestMissing
        }
    }

    private func makeTemporaryDirectory() throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("muxy-backup-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    private var currentAppVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "unknown"
    }

    private func backupStamp() -> String {
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd-HHmmss"
        formatter.locale = Locale(identifier: "en_US_POSIX")
        return formatter.string(from: Date())
    }
}
