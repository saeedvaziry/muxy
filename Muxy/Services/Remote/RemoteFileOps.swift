import Foundation

protocol RemoteFileOps: Sendable {
    func makeDirectory(at path: String) async throws
    func removeItem(at path: String) async throws
    func removeItem(at path: String, timeout: TimeInterval) async throws
    func exists(at path: String) async -> Bool
    func exists(at path: String, timeout: TimeInterval) async throws -> Bool
}

struct LocalFileOps: RemoteFileOps {
    func makeDirectory(at path: String) async throws {
        try await GitProcessRunner.offMainThrowing {
            try FileManager.default.createDirectory(
                atPath: path,
                withIntermediateDirectories: true,
                attributes: nil
            )
        }
    }

    func removeItem(at path: String) async throws {
        try await GitProcessRunner.offMainThrowing {
            try FileManager.default.removeItem(atPath: path)
        }
    }

    func removeItem(at path: String, timeout: TimeInterval) async throws {
        let result = try await SubprocessRunner.run(SubprocessRequest(
            executablePath: "/bin/rm",
            arguments: ["-rf", "--", path],
            timeout: timeout
        ))
        guard result.status == 0 else {
            throw RemoteFileOpsError.commandFailed(result.stderr.isEmpty ? path : result.stderr)
        }
    }

    func exists(at path: String) async -> Bool {
        await GitProcessRunner.offMain {
            FileManager.default.fileExists(atPath: path) ||
                (try? FileManager.default.destinationOfSymbolicLink(atPath: path)) != nil
        }
    }

    func exists(at path: String, timeout: TimeInterval) async throws -> Bool {
        let result = try await SubprocessRunner.run(SubprocessRequest(
            executablePath: "/bin/sh",
            arguments: [
                "-c",
                """
                if [ -e "$1" ] || [ -L "$1" ]; then exit 0; fi
                case "$1" in
                  */*) parent=${1%/*}; [ -n "$parent" ] || parent=/ ;;
                  *) parent=. ;;
                esac
                [ ! -e "$parent" ] && exit 1
                [ -x "$parent" ] && exit 1
                exit 2
                """,
                "muxy-path-exists",
                path,
            ],
            timeout: timeout
        ))
        return result.status != 1
    }
}

struct SSHFileOps: RemoteFileOps {
    let destination: SSHDestination

    func makeDirectory(at path: String) async throws {
        try await run("mkdir -p \(RemoteCommandBuilder.quoteRemotePath(path))")
    }

    func removeItem(at path: String) async throws {
        try await run("rm -rf \(RemoteCommandBuilder.quoteRemotePath(path))")
    }

    func removeItem(at path: String, timeout: TimeInterval) async throws {
        try await run("rm -rf \(RemoteCommandBuilder.quoteRemotePath(path))", timeout: timeout)
    }

    func exists(at path: String) async -> Bool {
        let result = try? await SSHCommandRunner.run(
            destination: destination,
            remoteCommand: Self.presenceCommand(path: path)
        )
        return result?.status != 1
    }

    func exists(at path: String, timeout: TimeInterval) async throws -> Bool {
        let result = try await SSHCommandRunner.run(
            destination: destination,
            remoteCommand: Self.presenceCommand(path: path),
            timeout: timeout
        )
        return result.status != 1
    }

    static func presenceCommand(path: String) -> String {
        let quotedPath = RemoteCommandBuilder.quoteRemotePath(path)
        return """
        __muxy_path=\(quotedPath)
        if [ -e "$__muxy_path" ] || [ -L "$__muxy_path" ]; then exit 0; fi
        case "$__muxy_path" in
          */*) __muxy_probe=${__muxy_path%/*}; [ -n "$__muxy_probe" ] || __muxy_probe=/ ;;
          *) __muxy_probe=. ;;
        esac
        while [ ! -e "$__muxy_probe" ] && [ ! -L "$__muxy_probe" ]; do
          [ "$__muxy_probe" = / ] && exit 2
          case "$__muxy_probe" in
            */*) __muxy_parent=${__muxy_probe%/*}; [ -n "$__muxy_parent" ] || __muxy_parent=/ ;;
            *) __muxy_parent=. ;;
          esac
          [ "$__muxy_parent" = "$__muxy_probe" ] && exit 2
          __muxy_probe=$__muxy_parent
        done
        [ -x "$__muxy_probe" ] && exit 1
        exit 2
        """
    }

    private func run(_ remoteCommand: String, timeout: TimeInterval = SSHCommandRunner.defaultTimeout) async throws {
        let result = try await SSHCommandRunner.run(
            destination: destination,
            remoteCommand: remoteCommand,
            timeout: timeout
        )
        guard result.status == 0 else {
            throw RemoteFileOpsError.commandFailed(result.stderr.isEmpty ? remoteCommand : result.stderr)
        }
    }
}

enum RemoteFileOpsError: LocalizedError {
    case commandFailed(String)

    var errorDescription: String? {
        switch self {
        case let .commandFailed(message): message
        }
    }
}

extension WorkspaceContext {
    var fileOps: any RemoteFileOps {
        switch self {
        case .local: LocalFileOps()
        case let .ssh(destination): SSHFileOps(destination: destination)
        }
    }
}
