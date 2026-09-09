import Foundation
import Testing

@testable import Muxy

@Suite("ExtensionManifest tabTypes and actions")
struct ExtensionTabManifestTests {
    @Test("commands default to event action")
    func commandsDefaultToEvent() throws {
        let manifest = try decode("""
        {
          "name": "x",
          "version": "1.0",
          "commands": [{ "id": "ping", "title": "Ping" }]
        }
        """)
        #expect(manifest.commands.first?.action == .event)
    }

    @Test("openTab action decodes tabType and data")
    func openTabActionDecodes() throws {
        let manifest = try decode("""
        {
          "name": "x",
          "version": "1.0",
          "tabTypes": [{ "id": "viewer", "title": "Viewer", "entry": "tabs/x.html" }],
          "commands": [{
            "id": "open",
            "title": "Open",
            "action": { "kind": "openTab", "tabType": "viewer", "data": { "n": 1 } }
          }]
        }
        """)
        guard case let .openTab(tabType, data) = manifest.commands.first?.action else {
            Issue.record("expected openTab action")
            return
        }
        #expect(tabType == "viewer")
        #expect(data == .object(["n": .number(1)]))
    }

    @Test("runScript action decodes script path")
    func runScriptActionDecodes() throws {
        let manifest = try decode("""
        {
          "name": "x",
          "version": "1.0",
          "commands": [{
            "id": "go",
            "title": "Go",
            "action": { "kind": "runScript", "script": "scripts/go.js" }
          }]
        }
        """)
        guard case let .runScript(script) = manifest.commands.first?.action else {
            Issue.record("expected runScript action")
            return
        }
        #expect(script == "scripts/go.js")
    }

    @Test("tabTypes decode with defaultData")
    func tabTypesDecode() throws {
        let manifest = try decode("""
        {
          "name": "x",
          "version": "1.0",
          "tabTypes": [{
            "id": "viewer",
            "title": "Viewer",
            "entry": "tabs/x.html",
            "defaultData": { "mode": "dark" }
          }]
        }
        """)
        let tabType = try #require(manifest.tabType(id: "viewer"))
        #expect(tabType.title == "Viewer")
        #expect(tabType.entry == "tabs/x.html")
        #expect(tabType.defaultData == .object(["mode": .string("dark")]))
    }

    private func decode(_ json: String) throws -> ExtensionManifest {
        try JSONDecoder().decode(ExtensionManifest.self, from: Data(json.utf8))
    }
}

@Suite("ExtensionManifestLoader validation for tabTypes")
struct ExtensionTabValidationTests {
    @Test("loader rejects duplicate tabType ids")
    func loaderRejectsDuplicateTabTypes() throws {
        let directory = try makeExtension(manifest: """
        {
          "name": "dupe",
          "version": "1.0",
          "tabTypes": [
            { "id": "viewer", "title": "A", "entry": "a.html" },
            { "id": "viewer", "title": "B", "entry": "b.html" }
          ]
        }
        """, extraFiles: ["a.html": "<a/>", "b.html": "<b/>"])
        defer { try? FileManager.default.removeItem(at: directory) }

        #expect(throws: ExtensionLoadError.self) {
            try ExtensionManifestLoader.load(from: directory)
        }
    }

    @Test("loader rejects openTab actions referencing unknown tab type")
    func loaderRejectsUnknownTabType() throws {
        let directory = try makeExtension(manifest: """
        {
          "name": "x",
          "version": "1.0",
          "commands": [{
            "id": "bad",
            "title": "Bad",
            "action": { "kind": "openTab", "tabType": "missing" }
          }]
        }
        """)
        defer { try? FileManager.default.removeItem(at: directory) }

        #expect(throws: ExtensionLoadError.self) {
            try ExtensionManifestLoader.load(from: directory)
        }
    }

    @Test("loader rejects tabType entry outside extension directory")
    func loaderRejectsEntryOutsideDirectory() throws {
        let directory = try makeExtension(manifest: """
        {
          "name": "esc",
          "version": "1.0",
          "tabTypes": [{ "id": "v", "title": "V", "entry": "../escape.html" }]
        }
        """)
        defer { try? FileManager.default.removeItem(at: directory) }

        #expect(throws: ExtensionLoadError.self) {
            try ExtensionManifestLoader.load(from: directory)
        }
    }

    @Test("loader rejects tabType entry that does not exist")
    func loaderRejectsMissingEntry() throws {
        let directory = try makeExtension(manifest: """
        {
          "name": "missing",
          "version": "1.0",
          "tabTypes": [{ "id": "v", "title": "V", "entry": "tabs/nope.html" }]
        }
        """)
        defer { try? FileManager.default.removeItem(at: directory) }

        #expect(throws: ExtensionLoadError.self) {
            try ExtensionManifestLoader.load(from: directory)
        }
    }

    @Test("loader rejects runScript referencing missing script")
    func loaderRejectsMissingScript() throws {
        let directory = try makeExtension(manifest: """
        {
          "name": "noscript",
          "version": "1.0",
          "commands": [{
            "id": "go",
            "title": "Go",
            "action": { "kind": "runScript", "script": "scripts/missing.js" }
          }]
        }
        """)
        defer { try? FileManager.default.removeItem(at: directory) }

        #expect(throws: ExtensionLoadError.self) {
            try ExtensionManifestLoader.load(from: directory)
        }
    }

    @Test("loader accepts valid manifest with tabTypes and openTab command")
    func loaderAcceptsValidManifest() throws {
        let directory = try makeExtension(manifest: """
        {
          "name": "good",
          "version": "1.0",
          "tabTypes": [{ "id": "v", "title": "V", "entry": "tabs/v.html" }],
          "commands": [{
            "id": "open",
            "title": "Open",
            "action": { "kind": "openTab", "tabType": "v" }
          }]
        }
        """, extraFiles: ["tabs/v.html": "<html/>"])
        defer { try? FileManager.default.removeItem(at: directory) }

        let muxyExtension = try ExtensionManifestLoader.load(from: directory)
        #expect(muxyExtension.manifest.tabTypes.count == 1)
        guard case let .openTab(tabType, _) = muxyExtension.manifest.commands.first?.action else {
            Issue.record("expected openTab action")
            return
        }
        #expect(tabType == "v")
    }

    @Test("resolveResource returns nil for path traversal")
    func resolveResourceRejectsTraversal() throws {
        let directory = try makeExtension(manifest: """
        {
          "name": "trav",
          "version": "1.0"
        }
        """)
        defer { try? FileManager.default.removeItem(at: directory) }

        let muxyExtension = try ExtensionManifestLoader.load(from: directory)
        #expect(muxyExtension.resolveResource("../escape") == nil)
        #expect(muxyExtension.resolveResource("nested/../../escape") == nil)
        #expect(muxyExtension.resolveResource("tabs/page.html") != nil)
    }

    private func makeExtension(manifest: String, extraFiles: [String: String] = [:]) throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("ext-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try ExtensionManifestFixture.write(flatManifest: manifest, to: directory)
        for (relPath, contents) in extraFiles {
            let fileURL = directory.appendingPathComponent(relPath)
            try FileManager.default.createDirectory(
                at: fileURL.deletingLastPathComponent(),
                withIntermediateDirectories: true
            )
            try Data(contents.utf8).write(to: fileURL)
        }
        return directory
    }
}

@Suite("ExtensionTabState")
@MainActor
struct ExtensionTabStateTests {
    @Test("displayTitle prefers customTitle over defaultTitle")
    func displayTitlePrefersCustom() {
        let state = ExtensionTabState(
            extensionID: "ext",
            tabTypeID: "viewer",
            projectPath: "/tmp",
            defaultTitle: "Viewer"
        )
        #expect(state.displayTitle == "Viewer")
        state.customTitle = "Renamed"
        #expect(state.displayTitle == "Renamed")
    }

    @Test("data is preserved")
    func dataPreserved() {
        let state = ExtensionTabState(
            extensionID: "ext",
            tabTypeID: "viewer",
            projectPath: "/tmp",
            defaultTitle: "Viewer",
            data: .object(["n": .number(1)])
        )
        #expect(state.data == .object(["n": .number(1)]))
    }
}

@Suite("TerminalTab extensionWebView round-trip")
@MainActor
struct TerminalTabExtensionRoundTrip {
    @Test("extensionWebView tab snapshots its extensionID, tabTypeID, and data")
    func snapshotPreservesExtensionFields() {
        let state = ExtensionTabState(
            extensionID: "pr-tools",
            tabTypeID: "pr-viewer",
            projectPath: "/tmp/test",
            defaultTitle: "PR Viewer",
            data: .object(["prNumber": .number(42)])
        )
        let tab = TerminalTab(extensionState: state)
        let snapshot = tab.snapshot()

        #expect(snapshot.kind == .extensionWebView)
        #expect(snapshot.extensionID == "pr-tools")
        #expect(snapshot.extensionTabTypeID == "pr-viewer")
        #expect(snapshot.extensionTabData == .object(["prNumber": .number(42)]))
        #expect(snapshot.projectPath == "/tmp/test")
    }

    @Test("restored extensionWebView tab reconstructs the ExtensionTabState")
    func restoreReconstructsState() throws {
        let snapshot = TerminalTabSnapshot(
            kind: .extensionWebView,
            customTitle: nil,
            colorID: nil,
            isPinned: false,
            projectPath: "/tmp/test",
            paneTitle: "PR Viewer",
            extensionID: "pr-tools",
            extensionTabTypeID: "pr-viewer",
            extensionTabData: .object(["prNumber": .number(7)])
        )
        let tab = TerminalTab(restoring: snapshot)
        let restored = try #require(tab.content.extensionState)
        #expect(restored.extensionID == "pr-tools")
        #expect(restored.tabTypeID == "pr-viewer")
        #expect(restored.defaultTitle == "PR Viewer")
        #expect(restored.data == .object(["prNumber": .number(7)]))
    }

    @Test("restored extensionWebView falls back to terminal when fields missing")
    func restoreFallsBackWhenMissingFields() {
        let snapshot = TerminalTabSnapshot(
            kind: .extensionWebView,
            customTitle: nil,
            colorID: nil,
            isPinned: false,
            projectPath: "/tmp/test",
            paneTitle: "Lost"
        )
        let tab = TerminalTab(restoring: snapshot)
        #expect(tab.kind == .terminal)
    }
}

@Suite("ExtensionLoadError messages for tab/script cases")
struct ExtensionLoadErrorTabMessages {
    @Test("messages cover tab type and script errors")
    func messagesCoverTabAndScriptErrors() {
        let entryURL = URL(fileURLWithPath: "/tmp/ext/tabs/x.html")
        let scriptURL = URL(fileURLWithPath: "/tmp/ext/scripts/x.js")

        let missing = ExtensionLoadError.tabTypeEntryMissing(tabTypeID: "v", url: entryURL)
        #expect(missing.errorDescription?.contains("v") == true)
        #expect(missing.errorDescription?.contains("tabs/x.html") == true)

        let outside = ExtensionLoadError.tabTypeEntryOutsideDirectory(tabTypeID: "v", url: entryURL)
        #expect(outside.errorDescription?.contains("escapes") == true)

        let dup = ExtensionLoadError.duplicateTabType("v")
        #expect(dup.errorDescription?.contains("v") == true)

        let unknown = ExtensionLoadError.commandReferencesUnknownTabType(commandID: "c", tabType: "missing")
        #expect(unknown.errorDescription?.contains("c") == true)
        #expect(unknown.errorDescription?.contains("missing") == true)

        let scriptMissing = ExtensionLoadError.scriptMissing(commandID: "c", url: scriptURL)
        #expect(scriptMissing.errorDescription?.contains("c") == true)
        #expect(scriptMissing.errorDescription?.contains("scripts/x.js") == true)

        let scriptOutside = ExtensionLoadError.scriptOutsideDirectory(commandID: "c", url: scriptURL)
        #expect(scriptOutside.errorDescription?.contains("escapes") == true)
    }
}

@Suite("ExtensionJSON and CommandAction round-trip")
struct ExtensionJSONRoundTripTests {
    @Test("ExtensionJSON round-trips all variants")
    func extensionJSONRoundTrip() throws {
        let value: ExtensionJSON = .object([
            "n": .null,
            "b": .bool(true),
            "i": .number(7),
            "s": .string("hi"),
            "a": .array([.number(1), .string("x")]),
        ])
        let data = try JSONEncoder().encode(value)
        let decoded = try JSONDecoder().decode(ExtensionJSON.self, from: data)
        #expect(decoded == value)
    }

    @Test("ExtensionCommandAction encodes and decodes each kind")
    func commandActionRoundTrip() throws {
        let actions: [ExtensionCommandAction] = [
            .event,
            .openTab(tabType: "viewer", data: .object(["k": .string("v")])),
            .runScript(script: "scripts/x.js"),
        ]
        for action in actions {
            let data = try JSONEncoder().encode(action)
            let decoded = try JSONDecoder().decode(ExtensionCommandAction.self, from: data)
            #expect(decoded == action)
        }
    }
}

@Suite("OpenTabRequest decoding")
struct OpenTabRequestTests {
    @Test("decodes terminal kind without payload")
    func terminalRequest() throws {
        let request = try decode("""
        { "kind": "terminal" }
        """)
        #expect(request.kind == .terminal)
        #expect(request.extensionPayload == nil)
    }

    @Test("decodes extensionWebView kind with extension payload")
    func extensionRequest() throws {
        let request = try decode("""
        {
          "kind": "extensionWebView",
          "extension": { "id": "pr-tools", "tabType": "pr-viewer", "data": { "prNumber": 42 } }
        }
        """)
        let payload = try #require(request.extensionPayload)
        #expect(payload.id == "pr-tools")
        #expect(payload.tabType == "pr-viewer")
        #expect(payload.data == .object(["prNumber": .number(42)]))
        #expect(payload.singleton == false)
    }

    @Test("decodes singleton flag on extension payload")
    func singletonRequest() throws {
        let request = try decode("""
        {
          "kind": "extensionWebView",
          "extension": { "id": "pr-tools", "tabType": "pr-viewer", "singleton": true }
        }
        """)
        let payload = try #require(request.extensionPayload)
        #expect(payload.singleton == true)
    }

    private func decode(_ json: String) throws -> OpenTabRequest {
        try JSONDecoder().decode(OpenTabRequest.self, from: Data(json.utf8))
    }
}
