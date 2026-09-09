import MuxyShared

public struct SessionReplayBuffer: Sendable {
    public let capacity: Int

    private var storage: [UInt8]
    private var start = 0
    private var count = 0
    private var hasDiscardedBytes = false
    private var alternateScreenActive = false
    private var screenControlTail: [UInt8] = []

    public init(capacity: Int) {
        self.capacity = max(capacity, 0)
        storage = [UInt8](repeating: 0, count: self.capacity)
    }

    public var isEmpty: Bool { byteCount == 0 }
    public var byteCount: Int { count }

    public mutating func append(_ bytes: [UInt8]) {
        bytes.withUnsafeBufferPointer { append($0) }
    }

    public mutating func append(_ bytes: UnsafeBufferPointer<UInt8>) {
        guard capacity > 0, !bytes.isEmpty else { return }
        appendForReplay(Array(bytes))
    }

    public var isAlternateScreenActive: Bool { alternateScreenActive }

    public var replayBytes: [UInt8] {
        guard !alternateScreenActive else { return [] }
        var output = bytes
        var needsLeadingFragmentCleanup = false
        if hasDiscardedBytes {
            let start = TerminalStreamSequence.safeReplayStart(in: output)
            needsLeadingFragmentCleanup = start == output.startIndex
            output = Array(output[start...])
        }
        if needsLeadingFragmentCleanup {
            let leading = TerminalStreamSequence.leadingSafeIndex(in: output)
            guard leading < output.count else { return [] }
            output = Array(output[leading...])
        }
        let trailing = TerminalStreamSequence.trailingSafeEnd(in: output)
        guard trailing > 0 else { return [] }
        return TerminalStreamSequence.removingResponseQueries(from: Array(output[..<trailing]))
    }

    private mutating func appendForReplay(_ bytes: [UInt8]) {
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
                    return
                }
                removeAll()
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
                if unhandledNewByteIndex < bytes.count {
                    appendStorage(Array(bytes[unhandledNewByteIndex...]))
                }
                updateScreenControlTail(from: combined)
                return
            }
            removeAll()
            alternateScreenActive = true
            unhandledNewByteIndex = max(unhandledNewByteIndex, max(0, next.upperBound - newByteOffset))
            index = next.upperBound
        }
        if !alternateScreenActive, unhandledNewByteIndex < bytes.count {
            appendStorage(Array(bytes[unhandledNewByteIndex...]))
        }
        updateScreenControlTail(from: combined)
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

    public var bytes: [UInt8] {
        guard !isEmpty else { return [] }
        var result = [UInt8]()
        result.reserveCapacity(count)
        for offset in 0 ..< count {
            result.append(storage[(start + offset) % capacity])
        }
        return result
    }

    public mutating func removeAll() {
        start = 0
        count = 0
        hasDiscardedBytes = false
        alternateScreenActive = false
        screenControlTail = []
    }

    private mutating func updateScreenControlTail(from bytes: [UInt8]) {
        screenControlTail = Array(bytes.suffix(TerminalStreamSequence.screenControlTailLength))
    }
}
