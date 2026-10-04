# extend.computer

[extend.computer](https://extend.computer) shares keyboard and mouse control between Macs. A shared Rust engine powers the CLI and Tauri desktop app; native macOS helpers capture and inject input.

The app supports encrypted local pairing, pinned device identities, account-connected devices, local-network connections with internet relay fallback, screen-edge handoff, native permission guidance, and transport recovery. Screen extension, mirroring, and remote desktop display streaming are unavailable.

## Documentation

**[Start reading the technical documentation →](docs/README.md)**

The documentation follows the system from its architecture through pairing, routing, input sessions, permissions, development, and releases. Each current chapter has previous/next navigation. [Proposals and historical research](docs/archived/README.md) are kept separately.

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
