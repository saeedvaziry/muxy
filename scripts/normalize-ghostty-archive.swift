#!/usr/bin/env swift
import Foundation

enum ArchiveNormalizationError: Error {
    case invalidArchive
    case invalidMember
    case unsupportedFormat
}

func unsignedInteger(in data: Data, at offset: Int, byteCount: Int) throws -> UInt64 {
    guard offset >= 0, byteCount > 0, offset + byteCount <= data.count else {
        throw ArchiveNormalizationError.invalidArchive
    }
    return data[offset ..< offset + byteCount].reduce(0) { ($0 << 8) | UInt64($1) }
}

func archiveRange(offset: UInt64, size: UInt64, dataCount: Int) throws -> (offset: Int, size: Int) {
    guard let offset = Int(exactly: offset),
          let size = Int(exactly: size),
          offset <= dataCount,
          size <= dataCount - offset
    else {
        throw ArchiveNormalizationError.invalidArchive
    }
    return (offset, size)
}

func fatArchiveRanges(in data: Data, count: Int, entrySize: Int, uses64BitRanges: Bool) throws -> [(offset: Int, size: Int)] {
    guard data.count >= 8, count <= (data.count - 8) / entrySize else {
        throw ArchiveNormalizationError.invalidArchive
    }
    return try (0 ..< count).map { index in
        let entry = 8 + index * entrySize
        let byteCount = uses64BitRanges ? 8 : 4
        return try archiveRange(
            offset: unsignedInteger(in: data, at: entry + 8, byteCount: byteCount),
            size: unsignedInteger(in: data, at: entry + 8 + byteCount, byteCount: byteCount),
            dataCount: data.count
        )
    }
}

func archiveRanges(in data: Data) throws -> [(offset: Int, size: Int)] {
    let archiveMagic = Data("!<arch>\n".utf8)
    if data.starts(with: archiveMagic) {
        return [(0, data.count)]
    }

    let magic = try unsignedInteger(in: data, at: 0, byteCount: 4)
    guard let count = Int(exactly: try unsignedInteger(in: data, at: 4, byteCount: 4)) else {
        throw ArchiveNormalizationError.invalidArchive
    }
    switch magic {
    case 0xCAFEBABE:
        return try fatArchiveRanges(in: data, count: count, entrySize: 20, uses64BitRanges: false)
    case 0xCAFEBABF:
        return try fatArchiveRanges(in: data, count: count, entrySize: 32, uses64BitRanges: true)
    default:
        throw ArchiveNormalizationError.unsupportedFormat
    }
}

struct ArchiveMember {
    let name: String
    let nameOffset: Int
    let nameLength: Int
    let namePadding: UInt8
}

func members(in data: Data, offset: Int, size: Int) throws -> [ArchiveMember] {
    let archiveMagic = Data("!<arch>\n".utf8)
    guard offset >= 0, size >= archiveMagic.count, offset + size <= data.count else {
        throw ArchiveNormalizationError.invalidArchive
    }
    guard data[offset ..< offset + archiveMagic.count] == archiveMagic else {
        throw ArchiveNormalizationError.invalidArchive
    }

    var members: [ArchiveMember] = []
    var position = offset + archiveMagic.count
    let end = offset + size
    while position + 60 <= end {
        guard data[position + 58] == 0x60, data[position + 59] == 0x0A else {
            throw ArchiveNormalizationError.invalidMember
        }
        let sizeData = data[position + 48 ..< position + 58]
        guard let sizeString = String(data: sizeData, encoding: .ascii),
              let memberSize = Int(sizeString.trimmingCharacters(in: .whitespaces)),
              memberSize >= 0,
              position + 60 + memberSize <= end
        else {
            throw ArchiveNormalizationError.invalidMember
        }
        let headerNameData = data[position ..< position + 16]
        guard let rawHeaderName = String(data: headerNameData, encoding: .ascii) else {
            throw ArchiveNormalizationError.invalidMember
        }
        let trimmedHeaderName = rawHeaderName.trimmingCharacters(in: .whitespaces)
        if trimmedHeaderName.hasPrefix("#1/"), let nameLength = Int(trimmedHeaderName.dropFirst(3)) {
            let nameOffset = position + 60
            guard nameLength > 0, nameLength <= memberSize, nameOffset + nameLength <= end else {
                throw ArchiveNormalizationError.invalidMember
            }
            let nameData = data[nameOffset ..< nameOffset + nameLength]
            guard let rawName = String(data: nameData, encoding: .ascii) else {
                throw ArchiveNormalizationError.invalidMember
            }
            members.append(ArchiveMember(
                name: rawName.trimmingCharacters(in: .controlCharacters),
                nameOffset: nameOffset,
                nameLength: nameLength,
                namePadding: 0
            ))
        } else {
            let name = trimmedHeaderName.hasSuffix("/") ? String(trimmedHeaderName.dropLast()) : trimmedHeaderName
            members.append(ArchiveMember(name: name, nameOffset: position, nameLength: 16, namePadding: 0x20))
        }
        position += 60 + memberSize
        if position.isMultiple(of: 2) == false {
            guard position < end, data[position] == 0x0A else {
                throw ArchiveNormalizationError.invalidMember
            }
            position += 1
        }
    }
    guard position == end else {
        throw ArchiveNormalizationError.invalidMember
    }
    return members
}

func normalizeArchive(at url: URL) throws -> Bool {
    var data = try Data(contentsOf: url)
    var changed = false
    for range in try archiveRanges(in: data) {
        let duplicates = try members(in: data, offset: range.offset, size: range.size)
            .filter { $0.name == "ext.o" }
        for (duplicateIndex, member) in duplicates.dropFirst().enumerated() {
            var replacement = Data("ext.\(duplicateIndex + 2).o".utf8)
            if member.namePadding == 0x20 {
                replacement.append(0x2F)
            }
            guard replacement.count <= member.nameLength else {
                throw ArchiveNormalizationError.invalidMember
            }
            replacement.append(contentsOf: repeatElement(member.namePadding, count: member.nameLength - replacement.count))
            data.replaceSubrange(member.nameOffset ..< member.nameOffset + member.nameLength, with: replacement)
            changed = true
        }
    }
    if changed {
        try data.write(to: url, options: .atomic)
    }
    return changed
}

do {
    guard let path = ProcessInfo.processInfo.arguments.dropFirst().first else {
        throw ArchiveNormalizationError.invalidArchive
    }
    let url = URL(fileURLWithPath: path)
    if try normalizeArchive(at: url) {
        print("==> Normalized duplicate object names in \(url.lastPathComponent)")
    }
} catch {
    FileHandle.standardError.write(Data("Failed to normalize Ghostty archive: \(error)\n".utf8))
    exit(1)
}
