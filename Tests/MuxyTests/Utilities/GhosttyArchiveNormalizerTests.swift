import Foundation
import Testing

@testable import Muxy

@Suite("Ghostty archive normalizer")
struct GhosttyArchiveNormalizerTests {
    @Test("renames duplicate object members and preserves payloads")
    func normalizesDuplicateMembers() throws {
        let members = makeArchive(payloads: [Data("first".utf8), Data("second".utf8)])
        let archive = try temporaryArchive(containing: makeFatArchive(containing: members))
        defer { try? FileManager.default.removeItem(at: archive.deletingLastPathComponent()) }

        let result = try runNormalizer(for: archive)
        let normalized = try Data(contentsOf: archive)
        let normalizedText = String(decoding: normalized, as: UTF8.self)

        #expect(result.status == 0)
        #expect(normalizedText.contains("ext.o/"))
        #expect(normalizedText.contains("ext.2.o/"))
        #expect(normalizedText.contains("first"))
        #expect(normalizedText.contains("second"))
    }

    @Test("is idempotent after normalization")
    func isIdempotent() throws {
        let archive = try temporaryArchive(containing: makeArchive(payloads: [Data("first".utf8), Data("second".utf8)]))
        defer { try? FileManager.default.removeItem(at: archive.deletingLastPathComponent()) }

        let firstResult = try runNormalizer(for: archive)
        let firstPass = try Data(contentsOf: archive)
        let secondResult = try runNormalizer(for: archive)
        let secondPass = try Data(contentsOf: archive)

        #expect(firstResult.status == 0)
        #expect(secondResult.status == 0)
        #expect(firstPass == secondPass)
        #expect(secondResult.output.isEmpty)
    }

    @Test("rejects malformed archives without modifying them")
    func preservesMalformedArchive() throws {
        let malformed = Data("!<arch>\next.o/".utf8)
        let archive = try temporaryArchive(containing: malformed)
        defer { try? FileManager.default.removeItem(at: archive.deletingLastPathComponent()) }

        let result = try runNormalizer(for: archive)

        #expect(result.status != 0)
        #expect(try Data(contentsOf: archive) == malformed)
    }

    @Test("rejects invalid member padding without modifying the archive")
    func preservesArchiveWithInvalidPadding() throws {
        var malformed = makeArchive(payloads: [Data("first".utf8)])
        malformed[malformed.count - 1] = 0
        let archive = try temporaryArchive(containing: malformed)
        defer { try? FileManager.default.removeItem(at: archive.deletingLastPathComponent()) }

        let result = try runNormalizer(for: archive)

        #expect(result.status != 0)
        #expect(try Data(contentsOf: archive) == malformed)
    }

    @Test("rejects malformed fat archive ranges without modifying them")
    func preservesMalformedFatArchives() throws {
        let members = makeArchive(payloads: [Data("first".utf8)])
        let malformedArchives = [
            makeFatArchive(containing: members, offset: .max),
            makeFatArchive(containing: members, size: .max),
            makeFatArchive(containing: members, size: UInt64(members.count + 1)),
            makeFatArchive(containing: Data()),
        ]

        for malformed in malformedArchives {
            let archive = try temporaryArchive(containing: malformed)
            defer { try? FileManager.default.removeItem(at: archive.deletingLastPathComponent()) }

            let result = try runNormalizer(for: archive)

            #expect(result.status != 0)
            #expect(try Data(contentsOf: archive) == malformed)
        }
    }

    private func makeArchive(payloads: [Data]) -> Data {
        var archive = Data("!<arch>\n".utf8)
        for payload in payloads {
            archive.append(Data(field("ext.o/", width: 16).utf8))
            archive.append(Data(field("0", width: 12).utf8))
            archive.append(Data(field("0", width: 6).utf8))
            archive.append(Data(field("0", width: 6).utf8))
            archive.append(Data(field("100644", width: 8).utf8))
            archive.append(Data(field(String(payload.count), width: 10).utf8))
            archive.append(Data("`\n".utf8))
            archive.append(payload)
            if payload.count.isMultiple(of: 2) == false {
                archive.append(0x0A)
            }
        }
        return archive
    }

    private func field(_ value: String, width: Int) -> String {
        value + String(repeating: " ", count: width - value.count)
    }

    private func makeFatArchive(
        containing archive: Data,
        offset: UInt64 = 40,
        size: UInt64? = nil
    ) -> Data {
        var result = Data()
        append(UInt32(0xCAFEBABF), to: &result)
        append(UInt32(1), to: &result)
        append(UInt32(0x0100000C), to: &result)
        append(UInt32(0), to: &result)
        append(offset, to: &result)
        append(size ?? UInt64(archive.count), to: &result)
        append(UInt32(0), to: &result)
        append(UInt32(0), to: &result)
        result.append(archive)
        return result
    }

    private func append<T: FixedWidthInteger>(_ value: T, to data: inout Data) {
        var bigEndian = value.bigEndian
        withUnsafeBytes(of: &bigEndian) { data.append(contentsOf: $0) }
    }

    private func temporaryArchive(containing data: Data) throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("ghostty-archive-normalizer-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let archive = directory.appendingPathComponent("ghostty-internal.a")
        try data.write(to: archive)
        return archive
    }

    private func runNormalizer(for archive: URL) throws -> (status: Int32, output: String) {
        let script = RepositoryRoot.find().appendingPathComponent("scripts/normalize-ghostty-archive.swift")
        let process = Process()
        let output = Pipe()
        process.executableURL = script
        process.arguments = [archive.path]
        process.standardOutput = output
        process.standardError = output

        try process.run()
        let data = output.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        return (process.terminationStatus, String(decoding: data, as: UTF8.self))
    }
}
