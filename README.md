# why

Explain why a file, command or listening TCP port is on your machine.
`why` reads symlinks, Git history, Nix store information and process details.
It does not change the target.

## Use

```sh
why ./Cargo.toml
why git
why :8080
why --help
```

Supply exactly one target:

- A path: show its symlink chain and the available Git or Nix information.
- A command: find it on `PATH`, then explain its path.
- A TCP port, with a leading colon: show its listener and parent processes.

A local path takes priority over a command with the same name.
Output uses colour only when standard output is a terminal.

## Build

Use Nix to build or run the program:

```sh
nix build
./result/bin/why --help
nix run . -- ./Cargo.toml
```

To build with Cargo in the Nix development environment:

```sh
nix develop -c cargo build --release --locked
./target/release/why --help
```

The flake provides packages for Linux and macOS on x86-64 and AArch64.
The Rust crate uses only the standard library.

## Runtime tools

Make these tools available on `PATH`:

- `git` for file history and the listener's working directory repository.
- `lsof` for TCP listeners and process working directories.
- `ps` for process details; macOS provides it, and Linux can use `procps`.
- `nix-store` for Nix derivations and garbage-collection roots, if you use Nix.
  Use the command from your existing Nix or Lix installation.

## Limitations

- Port targets inspect TCP listeners only, not UDP sockets.
- When a port has multiple listeners, the report uses the first listener.
- Process details depend on your permissions and can change during a report.
- Symlink and process chains stop after 16 steps.
- Nix inspection assumes the store is at `/nix/store`.
- Missing tools, command failures and empty output can produce the same report.
  For example, an `lsof` failure can report that no process listens on a port.
  A `nix-store` failure can report that a store path has no garbage-collection root.
- Reports show available history and process ancestry, not proof of intent.

## License

MIT. See [LICENSE](LICENSE).
