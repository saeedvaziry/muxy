import Foundation

public enum TerminalStreamSequence {
    public static let alternateScreenEnterSequences: [[UInt8]] = [
        [0x1B, 0x5B, 0x3F, 0x34, 0x37, 0x68],
        [0x1B, 0x5B, 0x3F, 0x31, 0x30, 0x34, 0x37, 0x68],
        [0x1B, 0x5B, 0x3F, 0x31, 0x30, 0x34, 0x39, 0x68],
    ]

    public static let alternateScreenLeaveSequences: [[UInt8]] = [
        [0x1B, 0x5B, 0x3F, 0x34, 0x37, 0x6C],
        [0x1B, 0x5B, 0x3F, 0x31, 0x30, 0x34, 0x37, 0x6C],
        [0x1B, 0x5B, 0x3F, 0x31, 0x30, 0x34, 0x39, 0x6C],
    ]

    public static let screenControlTailLength = max(
        alternateScreenEnterSequences.map(\.count).max() ?? 0,
        alternateScreenLeaveSequences.map(\.count).max() ?? 0
    )

    public static func safeReplayStart(in bytes: [UInt8]) -> Int {
        guard !bytes.isEmpty else { return 0 }
        guard let newline = bytes.firstIndex(where: { $0 == 0x0A || $0 == 0x0D }) else { return 0 }
        return bytes.index(after: newline)
    }

    public static func leadingSafeIndex(in bytes: [UInt8]) -> Int {
        var index = 0
        while index < bytes.count {
            let byte = bytes[index]
            if byte >= 0x80, byte <= 0xBF {
                index += 1
                continue
            }
            if byte == 0x5D {
                guard isLikelyBareOSCBody(in: bytes, from: index) else { return index }
                guard let end = oscTerminator(in: bytes, from: index + 1) else { return bytes.count }
                index = end
                continue
            }
            if byte == 0x5B {
                guard isLikelyBareCSIFragment(in: bytes, from: index) else { return index }
                guard let end = csiTerminator(in: bytes, from: index + 1) else { return bytes.count }
                index = end
                continue
            }
            return index
        }
        return index
    }

    public static func isLikelyBareOSCBody(in bytes: [UInt8], from index: Int) -> Bool {
        var cursor = index + 1
        var sawDigit = false
        while cursor < bytes.count, bytes[cursor] >= 0x30, bytes[cursor] <= 0x39 {
            sawDigit = true
            cursor += 1
        }
        return sawDigit && cursor < bytes.count && bytes[cursor] == 0x3B
    }

    public static func isLikelyBareCSIFragment(in bytes: [UInt8], from index: Int) -> Bool {
        let next = index + 1
        guard next < bytes.count else { return false }
        return (bytes[next] >= 0x30 && bytes[next] <= 0x3F) || (bytes[next] >= 0x20 && bytes[next] <= 0x2F)
    }

    public static func trailingSafeEnd(in bytes: [UInt8]) -> Int {
        var index = 0
        while index < bytes.count {
            guard bytes[index] == 0x1B else {
                index += 1
                continue
            }
            guard let end = escapeTerminator(in: bytes, from: index) else {
                return trailingUTF8SafeEnd(in: bytes, endingAt: index)
            }
            index = end
        }
        return trailingUTF8SafeEnd(in: bytes, endingAt: bytes.count)
    }

    public static func removingResponseQueries(from bytes: [UInt8]) -> [UInt8] {
        var output: [UInt8] = []
        output.reserveCapacity(bytes.count)
        var retainedStart = 0
        var index = 0
        while index < bytes.count {
            if bytes[index] == 0x05 {
                output.append(contentsOf: bytes[retainedStart ..< index])
                index += 1
                retainedStart = index
                continue
            }
            guard bytes[index] == 0x1B,
                  let end = escapeTerminator(in: bytes, from: index)
            else {
                index += 1
                continue
            }
            guard isResponseQuery(in: bytes, range: index ..< end) else {
                index = end
                continue
            }
            output.append(contentsOf: bytes[retainedStart ..< index])
            index = end
            retainedStart = end
        }
        output.append(contentsOf: bytes[retainedStart...])
        return output
    }

    public static func trailingUTF8SafeEnd(in bytes: [UInt8], endingAt end: Int) -> Int {
        guard end > 0 else { return 0 }
        var sequenceStart = end - 1
        var continuationCount = 0
        while sequenceStart > 0,
              isUTF8Continuation(bytes[sequenceStart]),
              continuationCount < 3
        {
            sequenceStart -= 1
            continuationCount += 1
        }
        guard let expectedLength = utf8SequenceLength(startingWith: bytes[sequenceStart]) else { return end }
        let actualLength = end - sequenceStart
        guard actualLength < expectedLength else { return end }
        guard bytes[(sequenceStart + 1) ..< end].allSatisfy(isUTF8Continuation) else { return end }
        return sequenceStart
    }

    public static func utf8SequenceLength(startingWith byte: UInt8) -> Int? {
        switch byte {
        case 0x00 ... 0x7F:
            1
        case 0xC2 ... 0xDF:
            2
        case 0xE0 ... 0xEF:
            3
        case 0xF0 ... 0xF4:
            4
        default:
            nil
        }
    }

    public static func isUTF8Continuation(_ byte: UInt8) -> Bool {
        byte >= 0x80 && byte <= 0xBF
    }

    public static func escapeTerminator(in bytes: [UInt8], from index: Int) -> Int? {
        let next = index + 1
        guard next < bytes.count else { return nil }
        switch bytes[next] {
        case 0x5D:
            return oscTerminator(in: bytes, from: next + 1)
        case 0x50,
             0x58,
             0x5E,
             0x5F:
            return stringControlTerminator(in: bytes, from: next + 1)
        case 0x5B:
            return csiTerminator(in: bytes, from: next + 1)
        case 0x20 ... 0x2F:
            var cursor = next + 1
            while cursor < bytes.count {
                if bytes[cursor] >= 0x30, bytes[cursor] <= 0x7E {
                    return cursor + 1
                }
                cursor += 1
            }
            return nil
        default:
            return next + 1
        }
    }

    public static func oscTerminator(in bytes: [UInt8], from index: Int) -> Int? {
        var cursor = index
        while cursor < bytes.count {
            if bytes[cursor] == 0x07 {
                return cursor + 1
            }
            if bytes[cursor] == 0x1B, cursor + 1 < bytes.count, bytes[cursor + 1] == 0x5C {
                return cursor + 2
            }
            cursor += 1
        }
        return nil
    }

    public static func stringControlTerminator(in bytes: [UInt8], from index: Int) -> Int? {
        var cursor = index
        while cursor + 1 < bytes.count {
            if bytes[cursor] == 0x1B, bytes[cursor + 1] == 0x5C {
                return cursor + 2
            }
            cursor += 1
        }
        return nil
    }

    public static func csiTerminator(in bytes: [UInt8], from index: Int) -> Int? {
        var cursor = index
        while cursor < bytes.count {
            if bytes[cursor] >= 0x40, bytes[cursor] <= 0x7E {
                return cursor + 1
            }
            cursor += 1
        }
        return nil
    }

    private static func isResponseQuery(in bytes: [UInt8], range: Range<Int>) -> Bool {
        guard range.count >= 2 else { return false }
        switch bytes[range.lowerBound + 1] {
        case 0x5A:
            return true
        case 0x5B:
            return isCSIResponseQuery(in: bytes, range: range)
        case 0x5D:
            return isOSCResponseQuery(in: bytes, range: range)
        case 0x50:
            return hasQueryPrefix(in: bytes, range: range, introducerLength: 2, prefixes: [[0x24, 0x71], [0x2B, 0x71]])
        case 0x5F:
            return isKittyGraphicsQuery(in: bytes, range: range)
        default:
            return false
        }
    }

    private static func isCSIResponseQuery(in bytes: [UInt8], range: Range<Int>) -> Bool {
        guard range.count >= 3 else { return false }
        let final = bytes[range.upperBound - 1]
        let body = bytes[(range.lowerBound + 2) ..< (range.upperBound - 1)]
        switch final {
        case 0x63,
             0x6E,
             0x78:
            return true
        case 0x70,
             0x77,
             0x75:
            return body.last == 0x24 || (final == 0x75 && body.elementsEqual([0x3F]))
        case 0x71:
            return body.first == 0x3E
        case 0x74:
            return isWindowReportQuery(body)
        case 0x79:
            return body.last == 0x2A
        case 0x53:
            return isGraphicsAttributeQuery(body)
        default:
            return false
        }
    }

    private static func isWindowReportQuery(_ body: ArraySlice<UInt8>) -> Bool {
        let operation = body.prefix { $0 >= 0x30 && $0 <= 0x39 }
        guard !operation.isEmpty, let value = Int(String(decoding: operation, as: UTF8.self)) else { return false }
        return [11, 13, 14, 15, 16, 18, 19, 20, 21].contains(value)
    }

    private static func isGraphicsAttributeQuery(_ body: ArraySlice<UInt8>) -> Bool {
        guard body.first == 0x3F else { return false }
        let parameters = String(decoding: body.dropFirst(), as: UTF8.self).split(separator: ";", omittingEmptySubsequences: false)
        return parameters.count >= 2 && parameters[1] == "1"
    }

    private static func isOSCResponseQuery(in bytes: [UInt8], range: Range<Int>) -> Bool {
        let payloadStart = range.lowerBound + 2
        let payloadEnd = controlStringPayloadEnd(in: bytes, range: range)
        guard payloadStart < payloadEnd else { return false }
        let payload = bytes[payloadStart ..< payloadEnd]
        guard let separator = payload.firstIndex(of: 0x3B) else { return false }
        let command = payload[..<separator]
        guard !command.isEmpty, command.allSatisfy({ $0 >= 0x30 && $0 <= 0x39 }) else { return false }
        var fieldStart = payload.index(after: separator)
        while fieldStart <= payload.endIndex {
            let fieldEnd = payload[fieldStart...].firstIndex(of: 0x3B) ?? payload.endIndex
            if payload[fieldStart ..< fieldEnd].elementsEqual([0x3F]) {
                return true
            }
            guard fieldEnd < payload.endIndex else { return false }
            fieldStart = payload.index(after: fieldEnd)
        }
        return false
    }

    private static func hasQueryPrefix(
        in bytes: [UInt8],
        range: Range<Int>,
        introducerLength: Int,
        prefixes: [[UInt8]]
    ) -> Bool {
        let payloadStart = range.lowerBound + introducerLength
        let payloadEnd = controlStringPayloadEnd(in: bytes, range: range)
        guard payloadStart < payloadEnd else { return false }
        let payload = bytes[payloadStart ..< payloadEnd]
        return prefixes.contains { payload.starts(with: $0) }
    }

    private static func isKittyGraphicsQuery(in bytes: [UInt8], range: Range<Int>) -> Bool {
        let payloadStart = range.lowerBound + 2
        let payloadEnd = controlStringPayloadEnd(in: bytes, range: range)
        guard payloadStart < payloadEnd, bytes[payloadStart] == 0x47 else { return false }
        let header = bytes[(payloadStart + 1) ..< payloadEnd].prefix { $0 != 0x3B }
        return String(decoding: header, as: UTF8.self)
            .split(separator: ",")
            .contains("a=q")
    }

    private static func controlStringPayloadEnd(in bytes: [UInt8], range: Range<Int>) -> Int {
        if bytes[range.upperBound - 1] == 0x07 {
            return range.upperBound - 1
        }
        return max(range.lowerBound, range.upperBound - 2)
    }

    public static func nextAlternateScreenSequence(in bytes: [UInt8], from index: Int, entering: Bool) -> Range<Int>? {
        let sequences = entering ? alternateScreenEnterSequences : alternateScreenLeaveSequences
        var best: Range<Int>?
        for sequence in sequences {
            guard let range = firstRange(of: sequence, in: bytes, from: index) else { continue }
            guard let current = best else {
                best = range
                continue
            }
            if range.lowerBound < current.lowerBound {
                best = range
            }
        }
        return best
    }

    public static func firstRange(of needle: [UInt8], in bytes: [UInt8], from index: Int) -> Range<Int>? {
        guard !needle.isEmpty, index >= 0, bytes.count - index >= needle.count else { return nil }
        var cursor = index
        while cursor + needle.count <= bytes.count {
            var offset = 0
            var matches = true
            while offset < needle.count {
                if bytes[cursor + offset] != needle[offset] {
                    matches = false
                    break
                }
                offset += 1
            }
            if matches {
                return cursor ..< cursor + needle.count
            }
            cursor += 1
        }
        return nil
    }
}
