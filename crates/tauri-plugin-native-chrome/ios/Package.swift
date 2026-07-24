// swift-tools-version:5.9

import PackageDescription

let package = Package(
    name: "tauri-plugin-native-chrome",
    platforms: [
        .iOS(.v14),
    ],
    products: [
        .library(
            name: "tauri-plugin-native-chrome",
            type: .static,
            targets: ["tauri-plugin-native-chrome"]
        ),
    ],
    dependencies: [
        .package(name: "Tauri", path: "../.tauri/tauri-api"),
    ],
    targets: [
        .target(
            name: "tauri-plugin-native-chrome",
            dependencies: [
                .byName(name: "Tauri"),
            ],
            path: "Sources"
        ),
    ]
)
