# extend.computer

[extend.computer](https://extend.computer) connects devices so they can share input and displays. Its product scope covers input sharing, display extension, screen mirroring, and remote desktop across desktop and mobile platforms. A shared Rust engine powers connection and trust management, with platform adapters for device capabilities.

> [!NOTE]
> Current support: keyboard and mouse sharing between macOS devices. Display extension, mirroring, remote desktop display streaming, and adapters for other platforms are not supported yet. See [sharing modes](docs/architecture/sharing-modes.md) and [platform support](docs/architecture/platforms.md).

Implemented foundations include encrypted local pairing, pinned device identities, account-connected devices, local-network connections with internet relay fallback, native permission guidance, and transport recovery.

## Documentation

**[Start reading the technical documentation →](docs/README.md)**

- [Contributing](CONTRIBUTING.md): development checks and contribution workflow.
- [Website](web/README.md): hosted account service and deployment.
- [Account server](server/README.md): standalone server setup and API.
- [Desktop changelog](desktop/CHANGELOG.md): release history.

## Development

```sh
npm run desktop:dev --prefix desktop
```

This builds and opens the explicitly signed development profile. Production builds retain Keychain identity storage and use the release signing pipeline. See [desktop development](docs/development/desktop.md) for prerequisites and profile boundaries.

Licensed under [MIT](LICENSE); vendored dependencies retain their own licenses.
