// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "ExtendComputerLowJitter",
    platforms: [.macOS(.v13)],
    products: [.library(name: "LowJitterLifecycle", targets: ["LowJitterLifecycle"])],
    targets: [
        .target(name: "LowJitterLifecycle"),
        .executableTarget(name: "LifecycleTests", dependencies: ["LowJitterLifecycle"], path: "Tests/LowJitterLifecycleTests")
    ]
)
