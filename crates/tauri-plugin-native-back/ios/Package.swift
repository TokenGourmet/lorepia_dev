// swift-tools-version:5.9

import PackageDescription

let package = Package(
    name: "tauri-plugin-native-back",
    platforms: [
        .iOS(.v14),
    ],
    products: [
        .library(
            name: "tauri-plugin-native-back",
            type: .static,
            targets: ["tauri-plugin-native-back"]
        ),
    ],
    dependencies: [
        .package(name: "Tauri", path: "../.tauri/tauri-api"),
    ],
    targets: [
        .target(
            name: "tauri-plugin-native-back",
            dependencies: [
                .byName(name: "Tauri"),
            ],
            path: "Sources"
        ),
    ]
)
