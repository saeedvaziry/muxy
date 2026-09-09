// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "Muxy",
    defaultLocalization: "en",
    platforms: [
        .macOS(.v14),
    ],
    products: [
        .library(name: "MuxyShared", targets: ["MuxyShared"]),
        .executable(name: "muxy-hook", targets: ["MuxyHookBridge"]),
        .executable(name: "muxy-session", targets: ["MuxySession"]),
    ],
    dependencies: [
        .package(url: "https://github.com/sparkle-project/Sparkle", exact: "2.9.1"),
        .package(url: "https://github.com/jpsim/Yams", from: "5.1.0"),
        .package(url: "https://github.com/getsentry/sentry-cocoa", from: "8.40.0"),
    ],
    targets: [
        .target(
            name: "MuxyShared",
            path: "MuxyShared"
        ),
        .executableTarget(
            name: "MuxyExtensionHost",
            dependencies: [
                "MuxyShared",
            ],
            path: "MuxyExtensionHost",
            exclude: ["MuxyExtensionHost.entitlements"],
            linkerSettings: [
                .linkedFramework("Foundation"),
                .linkedFramework("JavaScriptCore"),
            ]
        ),
        .target(
            name: "MuxyHookKit",
            dependencies: [
                "MuxyShared",
            ],
            path: "MuxyHookKit"
        ),
        .executableTarget(
            name: "MuxyHookBridge",
            dependencies: [
                "MuxyHookKit",
            ],
            path: "MuxyHookBridge"
        ),
        .target(
            name: "GhosttyKit",
            path: "GhosttyKit",
            publicHeadersPath: "."
        ),
        .target(
            name: "MuxySessionProtocol",
            dependencies: [
                "MuxyShared",
            ],
            path: "MuxySessionProtocol"
        ),
        .executableTarget(
            name: "MuxySession",
            dependencies: [
                "MuxySessionProtocol",
            ],
            path: "MuxySession"
        ),
        .target(
            name: "MuxyServer",
            dependencies: [
                "MuxyShared",
            ],
            path: "MuxyServer"
        ),
        .executableTarget(
            name: "Muxy",
            dependencies: [
                "GhosttyKit",
                "MuxyShared",
                "MuxyServer",
                "MuxySessionProtocol",
                .product(name: "Sparkle", package: "Sparkle"),
                .product(name: "Yams", package: "Yams"),
                .product(name: "Sentry", package: "sentry-cocoa"),
            ],
            path: "Muxy",
            exclude: ["Info.plist", "Muxy.entitlements"],
            resources: [
                .process("Resources/Assets.xcassets"),
                .process("Resources/Localization"),
                .copy("Resources/ProviderIcons"),
                .copy("Resources/ghostty"),
                .copy("Resources/ghostty-overrides"),
                .copy("Resources/scripts"),
                .copy("Resources/skills"),
                .copy("Resources/starter-kits"),
                .copy("Resources/terminfo"),
                .copy("Resources/tips.json"),
            ],
            linkerSettings: [
                .unsafeFlags([
                    "GhosttyKit.xcframework/macos-arm64_x86_64/ghostty-internal.a",
                ]),
                .linkedFramework("AppKit"),
                .linkedFramework("AVFoundation"),
                .linkedFramework("Carbon"),
                .linkedFramework("CoreAudio"),
                .linkedFramework("CoreGraphics"),
                .linkedFramework("CoreText"),
                .linkedFramework("Foundation"),
                .linkedFramework("IOKit"),
                .linkedFramework("Metal"),
                .linkedFramework("MetalKit"),
                .linkedFramework("QuartzCore"),
                .linkedFramework("Speech"),
                .linkedFramework("UserNotifications"),
                .linkedLibrary("c++"),
                .linkedLibrary("sqlite3"),
            ]
        ),
        .testTarget(
            name: "MuxyTests",
            dependencies: [
                "Muxy",
                "MuxyShared",
                "MuxyServer",
                "MuxySessionProtocol",
                "MuxyExtensionHost",
                "MuxyHookKit",
                .product(name: "Yams", package: "Yams"),
            ],
            path: "Tests/MuxyTests",
            linkerSettings: [
                .unsafeFlags([
                    "GhosttyKit.xcframework/macos-arm64_x86_64/ghostty-internal.a",
                ]),
                .linkedFramework("AppKit"),
                .linkedFramework("AVFoundation"),
                .linkedFramework("Carbon"),
                .linkedFramework("CoreAudio"),
                .linkedFramework("CoreGraphics"),
                .linkedFramework("CoreText"),
                .linkedFramework("Foundation"),
                .linkedFramework("IOKit"),
                .linkedFramework("Metal"),
                .linkedFramework("MetalKit"),
                .linkedFramework("QuartzCore"),
                .linkedFramework("Speech"),
                .linkedFramework("UserNotifications"),
                .linkedLibrary("c++"),
                .linkedLibrary("sqlite3"),
            ]
        ),
    ]
)
