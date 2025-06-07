// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "ArkavoReference",
    platforms: [
        .iOS(.v16)
    ],
    products: [
        .library(
            name: "ArkavoReference",
            targets: ["ArkavoReference"]
        ),
    ],
    targets: [
        .target(
            name: "ArkavoReference",
            dependencies: [],
            path: "Sources"
        ),
        .testTarget(
            name: "ArkavoReferenceTests",
            dependencies: ["ArkavoReference"],
            path: "Tests"
        ),
    ]
)