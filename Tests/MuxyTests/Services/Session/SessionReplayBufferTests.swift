import MuxySessionProtocol
import Testing

@Suite("SessionReplayBuffer")
struct SessionReplayBufferTests {
    @Test("starts empty")
    func startsEmpty() {
        let buffer = SessionReplayBuffer(capacity: 8)
        #expect(buffer.isEmpty)
        #expect(buffer.bytes.isEmpty)
        #expect(buffer.byteCount == 0)
    }

    @Test("keeps everything while below capacity")
    func keepsEverythingBelowCapacity() {
        var buffer = SessionReplayBuffer(capacity: 8)
        buffer.append([1, 2, 3])
        #expect(buffer.bytes == [1, 2, 3])
        buffer.append([4, 5])
        #expect(buffer.bytes == [1, 2, 3, 4, 5])
        #expect(buffer.byteCount == 5)
    }

    @Test("fills exactly to capacity without dropping")
    func fillsExactly() {
        var buffer = SessionReplayBuffer(capacity: 4)
        buffer.append([1, 2, 3, 4])
        #expect(buffer.bytes == [1, 2, 3, 4])
    }

    @Test("drops the oldest bytes once full")
    func dropsOldest() {
        var buffer = SessionReplayBuffer(capacity: 4)
        buffer.append([1, 2, 3, 4])
        buffer.append([5])
        #expect(buffer.bytes == [2, 3, 4, 5])
        buffer.append([6, 7])
        #expect(buffer.bytes == [4, 5, 6, 7])
    }

    @Test("keeps only the tail of a write larger than capacity")
    func keepsTailOfOversizedWrite() {
        var buffer = SessionReplayBuffer(capacity: 3)
        buffer.append([1, 2, 3, 4, 5, 6, 7])
        #expect(buffer.bytes == [5, 6, 7])
    }

    @Test("keeps only the tail of a write exactly one over capacity")
    func keepsTailOfBoundaryWrite() {
        var buffer = SessionReplayBuffer(capacity: 3)
        buffer.append([1, 2, 3, 4])
        #expect(buffer.bytes == [2, 3, 4])
    }

    @Test("stays correct across many wraparounds")
    func staysCorrectAcrossWraparounds() {
        var buffer = SessionReplayBuffer(capacity: 5)
        for value in UInt8(1) ... UInt8(50) {
            buffer.append([value])
        }
        #expect(buffer.bytes == [46, 47, 48, 49, 50])
    }

    @Test("wraps correctly when an oversized write follows a partial fill")
    func wrapsAfterPartialFill() {
        var buffer = SessionReplayBuffer(capacity: 4)
        buffer.append([1, 2, 3])
        buffer.append([4, 5, 6, 7, 8])
        #expect(buffer.bytes == [5, 6, 7, 8])
    }

    @Test("ignores empty writes and tolerates zero capacity")
    func toleratesDegenerateInput() {
        var buffer = SessionReplayBuffer(capacity: 4)
        buffer.append([])
        #expect(buffer.isEmpty)

        var zero = SessionReplayBuffer(capacity: 0)
        zero.append([1, 2, 3])
        #expect(zero.bytes.isEmpty)
    }

    @Test("clears back to empty")
    func clears() {
        var buffer = SessionReplayBuffer(capacity: 4)
        buffer.append([1, 2, 3, 4, 5])
        buffer.removeAll()
        #expect(buffer.isEmpty)
        buffer.append([9])
        #expect(buffer.bytes == [9])
    }

    @Test("removeAll exits alternate screen state")
    func removeAllExitsAlternateScreenState() {
        var buffer = SessionReplayBuffer(capacity: 64)
        buffer.append(Array("shell\n\u{1B}[?1049htui".utf8))
        #expect(buffer.isAlternateScreenActive)
        buffer.removeAll()
        #expect(!buffer.isAlternateScreenActive)
        #expect(buffer.replayBytes.isEmpty)
        buffer.append(Array("after".utf8))
        #expect(String(decoding: buffer.replayBytes, as: UTF8.self) == "after")
    }

    @Test("replay starts after a safe line boundary once bytes were discarded")
    func replayStartsAfterLineBoundaryWhenTruncated() {
        var buffer = SessionReplayBuffer(capacity: 6)
        buffer.append(Array("first\nsecond".utf8))
        #expect(String(decoding: buffer.replayBytes, as: UTF8.self) == "second")
    }

    @Test("replay drops leading utf8 continuation bytes")
    func replayDropsLeadingUTF8Continuation() {
        var buffer = SessionReplayBuffer(capacity: 4)
        buffer.append([0xE2, 0x82, 0xAC, 0x41, 0x42])
        #expect(buffer.replayBytes == [0x41, 0x42])
    }

    @Test("replay drops a leading bare osc body")
    func replayDropsLeadingBareOSCBody() {
        let payload = Array("]10;rgb:ffff/ffff/ffff\u{7}ready".utf8)
        var buffer = SessionReplayBuffer(capacity: payload.count)
        buffer.append(Array("prefix".utf8) + payload)
        #expect(String(decoding: buffer.replayBytes, as: UTF8.self) == "ready")
    }

    @Test("replay drops a leading bare csi parameter fragment")
    func replayDropsLeadingBareCSIParameterFragment() {
        let payload = Array("[?25lready".utf8)
        var buffer = SessionReplayBuffer(capacity: payload.count)
        buffer.append(Array("prefix".utf8) + payload)
        #expect(String(decoding: buffer.replayBytes, as: UTF8.self) == "ready")
    }

    @Test("wrapped replay drops a leading bare csi parameter fragment")
    func wrappedReplayDropsLeadingBareCSIParameterFragment() {
        let payload = Array("[?25lready".utf8)
        var buffer = SessionReplayBuffer(capacity: payload.count)
        buffer.append(Array("x".utf8))
        buffer.append(payload)
        #expect(String(decoding: buffer.replayBytes, as: UTF8.self) == "ready")
    }

    @Test("replay keeps leading bracket text when not truncated")
    func replayKeepsLeadingBracketTextWhenNotTruncated() {
        var buffer = SessionReplayBuffer(capacity: 64)
        buffer.append(Array("[notice] ready\n] prompt".utf8))
        #expect(String(decoding: buffer.replayBytes, as: UTF8.self) == "[notice] ready\n] prompt")
    }

    @Test("exact capacity first append keeps leading bracket text")
    func exactCapacityFirstAppendKeepsLeadingBracketText() {
        let payload = Array("[notice] ready".utf8)
        var buffer = SessionReplayBuffer(capacity: payload.count)
        buffer.append(payload)
        #expect(String(decoding: buffer.replayBytes, as: UTF8.self) == "[notice] ready")
    }

    @Test("exact capacity first append keeps leading csi-like text")
    func exactCapacityFirstAppendKeepsLeadingCSILikeText() {
        let payload = Array("[?25lready".utf8)
        var buffer = SessionReplayBuffer(capacity: payload.count)
        buffer.append(payload)
        #expect(buffer.replayBytes == payload)
    }

    @Test("truncated replay without newline keeps leading square bracket text")
    func truncatedReplayWithoutNewlineKeepsLeadingSquareBracketText() {
        let payload = Array("[notice] ready".utf8)
        var buffer = SessionReplayBuffer(capacity: payload.count)
        buffer.append(Array("prefix".utf8) + payload)
        #expect(String(decoding: buffer.replayBytes, as: UTF8.self) == "[notice] ready")
    }

    @Test("truncated replay without newline keeps leading osc bracket text")
    func truncatedReplayWithoutNewlineKeepsLeadingOSCBracketText() {
        let payload = Array("] prompt".utf8)
        var buffer = SessionReplayBuffer(capacity: payload.count)
        buffer.append(Array("prefix".utf8) + payload)
        #expect(String(decoding: buffer.replayBytes, as: UTF8.self) == "] prompt")
    }

    @Test("replay keeps leading bracket text after a safe line boundary")
    func replayKeepsLeadingBracketTextAfterSafeLineBoundary() {
        var buffer = SessionReplayBuffer(capacity: 18)
        buffer.append(Array("prefix\n[notice] ready".utf8))
        #expect(String(decoding: buffer.replayBytes, as: UTF8.self) == "[notice] ready")
    }

    @Test("replay drops an incomplete trailing escape sequence")
    func replayDropsIncompleteTrailingEscape() {
        var buffer = SessionReplayBuffer(capacity: 16)
        buffer.append(Array("ready\u{1B}]10;rgb".utf8))
        #expect(String(decoding: buffer.replayBytes, as: UTF8.self) == "ready")
    }

    @Test("replay drops incomplete trailing utf8 scalars")
    func replayDropsIncompleteTrailingUTF8Scalars() {
        let prefix = Array("ready".utf8)
        let incompleteScalars: [[UInt8]] = [
            [0xC2],
            [0xE2],
            [0xE2, 0x82],
            [0xF0],
            [0xF0, 0x9F],
            [0xF0, 0x9F, 0x98],
        ]
        for incompleteScalar in incompleteScalars {
            var buffer = SessionReplayBuffer(capacity: 16)
            buffer.append(prefix + incompleteScalar)
            #expect(buffer.replayBytes == prefix)
        }
    }

    @Test("replay keeps complete trailing utf8 scalars")
    func replayKeepsCompleteTrailingUTF8Scalars() {
        let prefix = Array("ready".utf8)
        let completeScalars: [[UInt8]] = [
            [0xC2, 0xA2],
            [0xE2, 0x82, 0xAC],
            [0xF0, 0x9F, 0x98, 0x80],
        ]
        for completeScalar in completeScalars {
            var buffer = SessionReplayBuffer(capacity: 16)
            buffer.append(prefix + completeScalar)
            #expect(buffer.replayBytes == prefix + completeScalar)
        }
    }

    @Test("replay sanitizes utf8 before an incomplete escape sequence")
    func replaySanitizesUTF8BeforeIncompleteEscape() {
        let prefix = Array("ready".utf8)
        let incompleteEscape = Array("\u{1B}]10;rgb".utf8)

        var incompleteScalar = SessionReplayBuffer(capacity: 32)
        incompleteScalar.append(prefix + [0xE2, 0x82] + incompleteEscape)
        #expect(incompleteScalar.replayBytes == prefix)

        let completeScalar: [UInt8] = [0xE2, 0x82, 0xAC]
        var completeScalarBuffer = SessionReplayBuffer(capacity: 32)
        completeScalarBuffer.append(prefix + completeScalar + incompleteEscape)
        #expect(completeScalarBuffer.replayBytes == prefix + completeScalar)
    }

    @Test("replay removes terminal queries that generate input responses")
    func replayRemovesTerminalResponseQueries() {
        let queries = [
            "\u{1B}[c",
            "\u{1B}[>q",
            "\u{1B}[?u",
            "\u{1B}[6n",
            "\u{1B}[?2026$p",
            "\u{1B}[14t",
            "\u{1B}]4;0;?\u{7}",
            "\u{1B}]10;?\u{1B}\\",
            "\u{1B}P+q544e\u{1B}\\",
            "\u{1B}P$qm\u{1B}\\",
            "\u{1B}_Ga=q,i=1,s=1,v=1,f=24;AAAA\u{1B}\\",
            "\u{1B}_Ga=t,i=1,s=1,v=1,f=24;AAAA\u{1B}\\",
            "\u{1B}_Ga=t,i=1,s=1,v=1,f=24,q=0;AAAA\u{1B}\\",
            "\u{1B}_Ga=t,i=1,s=1,v=1,f=24,q=1;AAAA\u{1B}\\",
            "\u{1B}Z",
        ].joined()
        let payload = Array(("before" + queries + "after").utf8)
        var buffer = SessionReplayBuffer(capacity: payload.count)
        buffer.append(payload)
        #expect(String(decoding: buffer.replayBytes, as: UTF8.self) == "beforeafter")
    }

    @Test("replay preserves terminal state changes")
    func replayPreservesTerminalStateChanges() {
        let payload = Array("before\u{1B}[31m\u{1B}[2J\u{1B}]0;?title\u{7}\u{1B}]4;0;rgb:ffff/0000/0000\u{7}\u{1B}_Ga=t,i=1,s=1,v=1,f=24,q=2;AAAA\u{1B}\\after".utf8)
        var buffer = SessionReplayBuffer(capacity: payload.count)
        buffer.append(payload)
        #expect(buffer.replayBytes == payload)
    }

    @Test("alternate screen suppresses replay until main screen output resumes")
    func alternateScreenSuppressesReplay() {
        var buffer = SessionReplayBuffer(capacity: 64)
        buffer.append(Array("shell\n\u{1B}[?1049htui".utf8))
        #expect(buffer.isAlternateScreenActive)
        #expect(buffer.replayBytes.isEmpty)
        #expect(buffer.bytes.isEmpty)
        buffer.append(Array("\u{1B}[?1049lafter".utf8))
        #expect(!buffer.isAlternateScreenActive)
        #expect(String(decoding: buffer.replayBytes, as: UTF8.self) == "after")
    }

    @Test("alternate screen detection spans appended chunks")
    func alternateScreenDetectionSpansChunks() {
        var buffer = SessionReplayBuffer(capacity: 64)
        buffer.append(Array("shell\n\u{1B}[?10".utf8))
        buffer.append(Array("49htui\u{1B}[?10".utf8))
        #expect(buffer.isAlternateScreenActive)
        #expect(buffer.replayBytes.isEmpty)
        buffer.append(Array("49lafter".utf8))
        #expect(!buffer.isAlternateScreenActive)
        #expect(String(decoding: buffer.replayBytes, as: UTF8.self) == "after")
    }

    @Test("alternate screen detection spans maximum length sequence prefix")
    func alternateScreenDetectionSpansMaximumLengthSequencePrefix() {
        var buffer = SessionReplayBuffer(capacity: 64)
        buffer.append(Array("shell\n\u{1B}[?1049".utf8))
        buffer.append(Array("htui".utf8))
        #expect(buffer.isAlternateScreenActive)
        #expect(buffer.replayBytes.isEmpty)
        buffer.append(Array("\u{1B}[?1049lafter".utf8))
        #expect(!buffer.isAlternateScreenActive)
        #expect(String(decoding: buffer.replayBytes, as: UTF8.self) == "after")
    }
}
