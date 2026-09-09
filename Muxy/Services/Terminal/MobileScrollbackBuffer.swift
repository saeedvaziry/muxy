import Foundation
import MuxyShared

struct MobileScrollbackBuffer: Sendable {
    private(set) var capacity: Int
    private var storage: [UInt8]
    private var start = 0
    private var count = 0
    private var hasDiscardedBytes = false
    private var alternateScreenActive = false
    private var screenControlTail: [UInt8] = []
    private var previousAppendStoredBytes = false

    init(capacity: Int) {
        self.capacity = max(capacity, 0)
        storage = [UInt8](repeating: 0, count: self.capacity)
    }

    var isEmpty: Bool { byteCount == 0 }
    var byteCount: Int { count }

    mutating func append(_ bytes: [UInt8], byteLimit: Int) {
        guard !bytes.isEmpty else { return }
        let storesBytes = byteLimit > 0
        if storesBytes {
            ensureCapacity(for: byteLimit)
        }
        previousAppendStoredBytes = appendForReplay(bytes, storesBytes: storesBytes)
    }

    mutating func trim(toByteLimit byteLimit: Int) {
        let target = max(byteLimit, 0)
        if capacity > 0, count > target {
            let drop = count - target
            start = (start + drop) % capacity
            count = target
            hasDiscardedBytes = true
        }
        let newCapacity = max(target, 1)
        guard newCapacity < capacity else { return }
        let current = bytes
        capacity = newCapacity
        storage = [UInt8](repeating: 0, count: capacity)
        start = 0
        count = current.count
        for (index, byte) in current.enumerated() {
            storage[index] = byte
        }
    }

    var isAlternateScreenActive: Bool { alternateScreenActive }

    var bytes: [UInt8] {
        guard !isEmpty else { return [] }
        var result = [UInt8]()
        result.reserveCapacity(count)
        for offset in 0 ..< count {
            result.append(storage[(start + offset) % capacity])
        }
        return result
    }

    var replayBytes: [UInt8] {
        var output = bytes
        guard !output.isEmpty else { return [] }
        var needsLeadingFragmentCleanup = false
        if hasDiscardedBytes {
            let replayStart = TerminalStreamSequence.safeReplayStart(in: output)
            needsLeadingFragmentCleanup = replayStart == output.startIndex
            if replayStart > 0 {
                output = Array(output[replayStart...])
            }
        }
        if needsLeadingFragmentCleanup {
            let leading = TerminalStreamSequence.leadingSafeIndex(in: output)
            guard leading < output.count else { return [] }
            output = Array(output[leading...])
        }
        let trailing = TerminalStreamSequence.trailingSafeEnd(in: output)
        guard trailing > 0 else { return [] }
        output = TerminalStreamSequence.removingResponseQueries(from: Array(output[..<trailing]))
        return MobileScrollbackBuffer.replayPrefix + output
    }

    mutating func removeAll() {
        start = 0
        count = 0
        hasDiscardedBytes = false
        alternateScreenActive = false
        screenControlTail = []
        previousAppendStoredBytes = false
    }

    private static let replayPrefix: [UInt8] = [
        0x1B, 0x5B, 0x30, 0x6D,
        0x1B, 0x28, 0x42,
        0x1B, 0x5B, 0x32, 0x4A,
        0x1B, 0x5B, 0x48,
    ]

    private mutating func ensureCapacity(for byteLimit: Int) {
        let target = max(byteLimit, 1)
        guard target > capacity else { return }
        let current = bytes
        let wasDiscarded = hasDiscardedBytes
        let wasAlt = alternateScreenActive
        let tail = screenControlTail
        capacity = target
        storage = [UInt8](repeating: 0, count: capacity)
        start = 0
        count = current.count
        for (index, byte) in current.enumerated() {
            storage[index] = byte
        }
        hasDiscardedBytes = wasDiscarded
        alternateScreenActive = wasAlt
        screenControlTail = tail
    }

    private mutating func appendForReplay(_ bytes: [UInt8], storesBytes: Bool) -> Bool {
        var didStore = false
        let previousTail = screenControlTail
        let combined = previousTail + bytes
        let newByteOffset = previousTail.count
        var index = 0
        var unhandledNewByteIndex = 0
        while index < combined.count {
            if alternateScreenActive {
                guard let leave = TerminalStreamSequence.nextAlternateScreenSequence(
                    in: combined,
                    from: index,
                    entering: false
                )
                else {
                    updateScreenControlTail(from: combined)
                    return didStore
                }
                if storesBytes {
                    appendStorage(Array(combined[leave]))
                    didStore = true
                } else {
                    balanceRetainedAltEnter()
                }
                alternateScreenActive = false
                unhandledNewByteIndex = max(unhandledNewByteIndex, max(0, leave.upperBound - newByteOffset))
                index = leave.upperBound
                continue
            }

            guard let next = TerminalStreamSequence.nextAlternateScreenSequence(
                in: combined,
                from: index,
                entering: true
            )
            else {
                if storesBytes, unhandledNewByteIndex < bytes.count {
                    appendStorage(Array(bytes[unhandledNewByteIndex...]))
                    didStore = true
                }
                updateScreenControlTail(from: combined)
                return didStore
            }
            let prefixEnd = min(max(next.lowerBound - newByteOffset, unhandledNewByteIndex), bytes.count)
            if storesBytes, unhandledNewByteIndex < prefixEnd {
                appendStorage(Array(bytes[unhandledNewByteIndex ..< prefixEnd]))
                didStore = true
            }
            let prefixRetained = next.lowerBound >= newByteOffset || previousAppendStoredBytes
            let retainedStart = max(next.lowerBound, newByteOffset)
            if storesBytes, prefixRetained, retainedStart < next.upperBound {
                appendStorage(Array(combined[retainedStart ..< next.upperBound]))
                didStore = true
            }
            alternateScreenActive = true
            unhandledNewByteIndex = max(unhandledNewByteIndex, max(0, next.upperBound - newByteOffset))
            index = next.upperBound
        }
        if storesBytes, !alternateScreenActive, unhandledNewByteIndex < bytes.count {
            appendStorage(Array(bytes[unhandledNewByteIndex...]))
            didStore = true
        }
        updateScreenControlTail(from: combined)
        return didStore
    }

    private mutating func appendStorage(_ bytes: [UInt8]) {
        guard capacity > 0, !bytes.isEmpty else { return }
        guard bytes.count <= capacity else {
            let tail = bytes.suffix(capacity)
            for (offset, byte) in tail.enumerated() {
                storage[offset] = byte
            }
            start = 0
            count = capacity
            hasDiscardedBytes = true
            return
        }
        if bytes.count == capacity {
            let discardedExistingBytes = !isEmpty
            for (offset, byte) in bytes.enumerated() {
                storage[offset] = byte
            }
            start = 0
            count = capacity
            if discardedExistingBytes {
                hasDiscardedBytes = true
            }
            return
        }
        for byte in bytes {
            storage[(start + count) % capacity] = byte
            if count < capacity {
                count += 1
            } else {
                start = (start + 1) % capacity
                hasDiscardedBytes = true
            }
        }
    }

    private mutating func updateScreenControlTail(from bytes: [UInt8]) {
        screenControlTail = Array(bytes.suffix(TerminalStreamSequence.screenControlTailLength))
    }

    private mutating func balanceRetainedAltEnter() {
        let stored = bytes
        guard let enterStart = lastEnterStart(in: stored) else { return }
        guard TerminalStreamSequence.nextAlternateScreenSequence(
            in: stored,
            from: enterStart + 1,
            entering: false
        ) == nil
        else { return }
        count = enterStart
        if isEmpty {
            start = 0
        }
    }

    private func lastEnterStart(in bytes: [UInt8]) -> Int? {
        var found: Int?
        var index = 0
        while index < bytes.count {
            guard let range = TerminalStreamSequence.nextAlternateScreenSequence(
                in: bytes,
                from: index,
                entering: true
            )
            else {
                index += 1
                continue
            }
            found = range.lowerBound
            index = range.upperBound
        }
        return found
    }
}
