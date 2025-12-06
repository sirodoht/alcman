# alcman

atproto-based book social network

## Setup

```sh
# Compile project (development mode)
cargo build

# Compile and serve on default port :3000
cargo run --bin alcmanserver -- --serve

# Serve with auto-reload
cargo watch -x 'run --bin alcmanserver -- --serve'

# Production mode
cargo run --release --bin alcmanserver -- --serve
```

The server will start at `http://localhost:3000`

## License

GNU Affero General Public License version 3
